import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { once } from "node:events";
import { mkdtempSync } from "node:fs";
import http from "node:http";
import { tmpdir } from "node:os";
import path from "node:path";
import test, { type TestContext } from "node:test";

import { parseConnectionsStatus, type AgentConnectionsStatus } from "@/lib/hosted-agent-controls";
import { inferenceView } from "@/lib/inference-status";

import {
  createInferenceFake,
  forwardRuntimeCommand,
  parseInferenceChoice,
  runtimeCommandsForwardUrl,
  type FakeAgentKind,
  type FakeInferenceRoute,
} from "../../scripts/web-design-fixture";

const MACHINE = "runtime-web-design-test";
const PHASE_MS = 1_000;
const PASTED_KEY = "sk-or-v1-fake-web-design-test-key";

type Fixture = {
  fake: ReturnType<typeof createInferenceFake>;
  tick(): void;
  replies: string[];
};

function installFixture(
  t: TestContext,
  options: { agent: FakeAgentKind; saved?: FakeInferenceRoute; unconfirmed?: boolean }
): Fixture {
  let clock = 1_790_000_000_000;
  const fake = createInferenceFake({ ...options, phaseMs: PHASE_MS, now: () => clock });
  const scratch = mkdtempSync(path.join(tmpdir(), "web-design-fixture-"));
  const env: Record<string, string> = {
    FC_WORKOS_AUTH_ENABLED: "",
    FC_DASHBOARD_ALLOW_DEV_ACCOUNT_AUTH: "1",
    FC_DASHBOARD_DEV_EMAIL: "fixture-user@example.test",
    FC_DASHBOARD_DEV_WORKOS_USER_ID: "user_web_design",
    FC_DASHBOARD_DEV_WORKOS_ACCESS_TOKEN: "fake-access-token",
    FC_CORE_BASE_URL: "https://core.test",
    FC_HOSTED_WEB_DEVICE_URL: "https://hwd.test",
    FINITECHAT_HOSTED_API_TOKEN: "fake-hwd-token",
    FC_REPO_ROOT: scratch,
    FC_WORKSPACE_ROOT: scratch,
    FC_FINITED_BIN: "",
    PATH: scratch,
  };
  const previous = Object.fromEntries(Object.keys(env).map((name) => [name, process.env[name]]));
  Object.assign(process.env, env);
  t.after(() => {
    for (const [name, value] of Object.entries(previous)) {
      if (value === undefined) delete process.env[name];
      else process.env[name] = value;
    }
  });

  const fixture: Fixture = {
    fake,
    tick() {
      clock += PHASE_MS;
    },
    replies: [],
  };
  t.mock.method(globalThis, "fetch", async (input: string | URL | Request, init: RequestInit = {}) => {
    const url = new URL(String(input instanceof Request ? input.url : input));
    if (url.href === "https://core.test/api/core/v1/me") {
      return Response.json({
        workos_user_id: "user_web_design",
        email: "fixture-user@example.test",
        projects: [{ project: { id: "project_web_design", display_name: "Moss" }, runtime: { id: MACHINE } }],
      });
    }
    if (url.href === "https://hwd.test/v1/app/agent-bindings/open") {
      return Response.json({ hosted_agent_binding: { canonical_room_id: "room_design", agent_account_id: "agent_design" } });
    }
    if (url.href === "https://hwd.test/v1/app/runtime-commands") {
      const reply = fake.runtimeCommand(JSON.parse(String(init.body ?? "")));
      fixture.replies.push(JSON.stringify(reply));
      return Response.json(reply);
    }
    throw new Error(`unexpected fetch ${url}`);
  });
  return fixture;
}

const routeContext = { params: Promise.resolve({ machineId: MACHINE }) };

async function post(payload: unknown) {
  const { POST } = await import("../app/api/connections/machines/[machineId]/route");
  const response = await POST(
    new Request(`https://dashboard.test/api/connections/machines/${MACHINE}`, {
      method: "POST",
      body: JSON.stringify(payload),
    }),
    routeContext
  );
  return { status: response.status, body: await response.json() };
}

async function readStatus() {
  const { GET } = await import("../app/api/connections/machines/[machineId]/route");
  const response = await GET(new Request(`https://dashboard.test/api/connections/machines/${MACHINE}`), routeContext);
  assert.equal(response.status, 200);
  const status = (await response.json()) as AgentConnectionsStatus;
  return { status, view: inferenceView(status) };
}

async function finishOperation(fixture: Fixture) {
  for (let step = 0; step < 8; step += 1) {
    fixture.tick();
    const operation = (await readStatus()).view.operation;
    assert.ok(operation);
    if (operation.state !== "running") return operation;
  }
  throw new Error("operation never finished");
}

test("fixture exposes the legacy, PR1, and unconfirmed browser states", () => {
  const status = (options: Parameters<typeof createInferenceFake>[0]) => {
    const reply = createInferenceFake(options).runtimeCommand({
      command: "agent.connections.status",
      schema: "finite.agent.empty.request.v1",
      body: {},
    });
    return inferenceView(parseConnectionsStatus(reply.body));
  };

  assert.deepStrictEqual(status({ agent: "pr1" }).saved, {
    route: "finite_private",
    provider: "custom",
    model: "glm-5-3-flash",
  });
  const unconfirmed = status({ agent: "pr1", saved: "openrouter", unconfirmed: true });
  assert.deepStrictEqual([unconfirmed.finitePrivate.state, unconfirmed.openrouter.state], ["unknown", "key_saved"]);
  assert.deepStrictEqual(status({ agent: "legacy", saved: "openai_codex" }).saved, {
    route: "openai_codex",
    provider: "openai-codex",
    model: "gpt-5.5",
  });

  assert.deepStrictEqual(parseInferenceChoice("pr1 openrouter unconfirmed"), {
    agent: "pr1",
    saved: "openrouter",
    unconfirmed: true,
  });
  for (const invalid of ["full", "pr1 openai_codex", "legacy openrouter extra"]) {
    assert.equal(parseInferenceChoice(invalid), null);
  }
});

test("fixture drives the browser's legacy apply and PR1 operation retry through real routes", async (t) => {
  const fixture = installFixture(t, { agent: "pr1" });
  let response = await post({ action: "inference", profile: "openrouter", apiKey: PASTED_KEY, model: "openai/gpt-5-mini" });
  assert.equal(response.status, 200);
  assert.equal((await readStatus()).view.openrouter.keyHash, createHash("sha256").update(PASTED_KEY).digest("hex"));

  fixture.fake.failNextOperation();
  response = await post({ action: "inference_select", route: "finite_private" });
  assert.equal(response.body.result.accepted, true);
  assert.equal((await finishOperation(fixture)).state, "failed");
  response = await post({ action: "inference_select", route: "finite_private" });
  assert.deepStrictEqual(response.body.result, { changed: false });
  assert.equal((await readStatus()).view.operation, null);

  response = await post({ action: "inference_select", route: "openrouter", model: "openai/gpt-5-mini" });
  assert.equal(response.body.result.accepted, true);
  assert.equal((await finishOperation(fixture)).state, "succeeded");

  fixture.fake.failNextOperation();
  response = await post({ action: "inference_disconnect", route: "openrouter" });
  const operationId = response.body.result.operation_id;
  const failed = await finishOperation(fixture);
  assert.deepStrictEqual([failed.state, failed.errorCode, failed.attempts], ["failed", "verify_failed", 3]);

  response = await post({ action: "inference_disconnect", route: "openrouter" });
  assert.equal(response.body.result.operation_id, operationId);
  assert.equal((await finishOperation(fixture)).state, "succeeded");
  assert.equal((await readStatus()).view.openrouter.state, "no_key");
  assert.ok(fixture.replies.every((reply) => !reply.includes(PASTED_KEY)));
});

test("runtime-command forwarding is loopback-only and preserves the authenticated request", async (t) => {
  for (const value of [
    "http://127.0.0.1",
    "http://127.0.0.1:0",
    "http://127.0.0.1:65536",
    "http://127.0.0.1:18080/path",
    "http://127.0.0.1:18080?query=1",
    "https://127.0.0.1:18080",
    "http://10.0.0.5:18080",
    "http://user:pass@localhost:18080",
    "http://127.0.0.1.example.com:18080",
    "http://localhost.:18080",
    "http://127.1:18080",
    "http://[::1]:18080",
    " http://127.0.0.1:18080",
  ]) {
    assert.throws(() => runtimeCommandsForwardUrl(value), /must be http:\/\/127\.0\.0\.1:<port> or http:\/\/localhost:<port>/u);
  }

  let received: { method?: string; path?: string; body: string; authorization?: string; user?: string } | undefined;
  const harness = http.createServer(async (request, response) => {
    const chunks: Buffer[] = [];
    for await (const chunk of request) chunks.push(chunk as Buffer);
    received = {
      method: request.method,
      path: request.url,
      body: Buffer.concat(chunks).toString("utf8"),
      authorization: request.headers.authorization,
      user: request.headers["x-finite-workos-user-id"] as string | undefined,
    };
    response.writeHead(409, { "content-type": "application/json" });
    response.end(JSON.stringify({ error: "synthetic upstream refusal" }));
  });
  harness.listen(0, "127.0.0.1");
  await once(harness, "listening");
  t.after(() => harness.close());
  const target = runtimeCommandsForwardUrl(`http://127.0.0.1:${(harness.address() as { port: number }).port}`);
  assert.ok(target);

  const fixture = http.createServer((request, response) => void forwardRuntimeCommand(target, request, response));
  fixture.listen(0, "127.0.0.1");
  await once(fixture, "listening");
  t.after(() => fixture.close());

  const body = JSON.stringify({ command: "agent.connections.status", schema: "finite.agent.empty.request.v1", body: {} });
  const response = await fetch(`http://127.0.0.1:${(fixture.address() as { port: number }).port}/v1/app/runtime-commands`, {
    method: "POST",
    headers: {
      authorization: "Bearer web-design-hosted-device-token",
      "x-finite-workos-user-id": "user_web_design",
      "content-type": "application/json",
    },
    body,
  });
  assert.equal(response.status, 409);
  assert.deepStrictEqual(await response.json(), { error: "synthetic upstream refusal" });
  assert.deepStrictEqual(received, {
    method: "POST",
    path: "/v1/app/runtime-commands",
    body,
    authorization: "Bearer web-design-hosted-device-token",
    user: "user_web_design",
  });
});
