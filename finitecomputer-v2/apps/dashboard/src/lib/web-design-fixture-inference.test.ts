import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { createHash } from "node:crypto";
import { once } from "node:events";
import { mkdtempSync } from "node:fs";
import http from "node:http";
import { tmpdir } from "node:os";
import path from "node:path";
import test, { type TestContext } from "node:test";

import { parseConnectionsStatus, type AgentConnectionsStatus } from "@/lib/hosted-agent-controls";
import { inferenceView, pollDelayMs, routeLabel } from "@/lib/inference-status";

import {
  createInferenceFake,
  forwardRuntimeCommand,
  parseInferenceChoice,
  runtimeCommandsForwardUrl,
  type FakeAgentKind,
  type FakeInferenceRoute,
} from "../../scripts/web-design-fixture";

// The design fixture's Connections fakes (DESIGN §10.7), driven through the real dashboard route,
// action functions, and parser. A fake reply the parser rejects or degrades fails here.

const MACHINE = "runtime-web-design-test";
const PHASE_MS = 1_000;
const START_MS = 1_790_000_000_000;
const PASTED_KEY = "sk-or-v1-fake-web-design-test-key";
const PR1_CAPABILITIES = ["inference.status.v2", "inference.select.v1", "inference.disconnect.v1"];

type Fixture = {
  fake: ReturnType<typeof createInferenceFake>;
  tick(ms?: number): void;
  commands: string[];
  replies: string[];
  logs: string[];
};

function installFixture(
  t: TestContext,
  options: { agent: FakeAgentKind; saved?: FakeInferenceRoute; unconfirmed?: boolean }
): Fixture {
  let clock = START_MS;
  const fake = createInferenceFake({ ...options, phaseMs: PHASE_MS, now: () => clock });
  const scratch = mkdtempSync(path.join(tmpdir(), "w4-fixture-"));
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
    tick(ms = PHASE_MS) {
      clock += ms;
    },
    commands: [],
    replies: [],
    logs: [],
  };
  for (const method of ["log", "info", "warn", "error", "debug"] as const) {
    t.mock.method(console, method, (...args: unknown[]) => {
      fixture.logs.push(args.map((arg) => String(arg)).join(" "));
    });
  }
  t.mock.method(globalThis, "fetch", async (input: string | URL | Request, init: RequestInit = {}) => {
    const url = new URL(String(input instanceof Request ? input.url : input));
    const body = typeof init.body === "string" ? init.body : "";
    if (url.href === "https://core.test/api/core/v1/me") {
      return Response.json({
        workos_user_id: "user_web_design",
        email: "fixture-user@example.test",
        projects: [{ project: { id: "project_web_design", display_name: "Moss" }, runtime: { id: MACHINE, contact_endpoint: null } }],
      });
    }
    if (url.href === "https://hwd.test/v1/app/agent-bindings/open") {
      return Response.json({ hosted_agent_binding: { canonical_room_id: "room_design", agent_account_id: "agent_design" } });
    }
    if (url.href === "https://hwd.test/v1/app/runtime-commands") {
      const request = JSON.parse(body) as { command: string };
      fixture.commands.push(request.command);
      const reply = fake.runtimeCommand(request);
      if (request.command === "agent.connections.status") assertParsesUnchanged(reply.body);
      fixture.replies.push(JSON.stringify(reply));
      return Response.json(reply);
    }
    throw new Error(`unexpected fetch ${url}`);
  });
  return fixture;
}

/** Every field the fake sends must come out of the real parser exactly as it went in. */
function assertParsesUnchanged(raw: unknown) {
  const sent = raw as { inference: unknown; capabilities?: unknown };
  const parsed = parseConnectionsStatus(raw);
  assert.deepStrictEqual(parsed.inference, sent.inference);
  assert.deepStrictEqual(parsed.capabilities, sent.capabilities);
}

const routeContext = { params: Promise.resolve({ machineId: MACHINE }) };

async function post(payload: unknown) {
  const { POST } = await import("../app/api/connections/machines/[machineId]/route");
  const response = await POST(
    new Request(`https://dashboard.test/api/connections/machines/${MACHINE}`, { method: "POST", body: JSON.stringify(payload) }),
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

async function readJson(route: "openrouter/usage" | "codex/models") {
  const handler =
    route === "openrouter/usage"
      ? await import("../app/api/connections/machines/[machineId]/openrouter/usage/route")
      : await import("../app/api/connections/machines/[machineId]/codex/models/route");
  const response = await handler.GET(new Request(`https://dashboard.test/api/connections/machines/${MACHINE}/${route}`), routeContext);
  return { status: response.status, body: await response.json() };
}

/** Lets time pass one phase at a time, and records what status shows, until the operation stops running. */
async function runOperation(fixture: Fixture) {
  const seen: string[] = [];
  for (let step = 0; step < 10; step += 1) {
    fixture.tick();
    const { view } = await readStatus();
    const operation = view.operation;
    assert.ok(operation);
    seen.push(operation.state === "running" ? operation.phase : `${operation.state}:${operation.phase}`);
    if (operation.state !== "running") return seen;
  }
  throw new Error("operation never finished");
}

const sha256 = (value: string) => createHash("sha256").update(value).digest("hex");

/** The fake's status read without the dashboard route, through the real parser. */
function fakeView(options: Parameters<typeof createInferenceFake>[0]) {
  const reply = createInferenceFake(options).runtimeCommand({
    command: "agent.connections.status",
    schema: "finite.agent.empty.request.v1",
    body: {},
  });
  assertParsesUnchanged(reply.body);
  return inferenceView(parseConnectionsStatus(reply.body));
}

test("T-W20 (fake): a PR1 agent's status parses unchanged and reads as stored facts", async (t) => {
  const fixture = installFixture(t, { agent: "pr1" });
  const { status, view } = await readStatus();
  assert.deepStrictEqual(status.capabilities, PR1_CAPABILITIES);
  assert.equal(view.v2, true);
  assert.deepStrictEqual(view.saved, { route: "finite_private", provider: "custom", model: "glm-5-3-flash" });
  assert.deepStrictEqual(view.finitePrivate, { state: "configured", reason: null });
  assert.equal(view.openrouter.state, "no_key");
  assert.equal(view.codex, null);
  assert.deepStrictEqual(view.fallback, { state: "configured", reason: null, model: "glm-5-3-flash", extraEntries: 0 });
  assert.equal(view.operation, null);
  assert.equal(pollDelayMs(view, true), null);
  assert.deepStrictEqual(status.inference.routes?.openai_codex, undefined);
  assert.deepStrictEqual(fixture.commands, ["agent.owner.claim", "agent.connections.status"]);
});

test("R34 (fake): a PR1 agent that couldn't confirm its facts reports them unknown, as agentd does after a failed helper read", async (t) => {
  const fixture = installFixture(t, { agent: "pr1", saved: "openrouter", unconfirmed: true });
  const { view } = await readStatus();
  assert.equal(view.v2, true);
  assert.deepStrictEqual(view.saved, { route: "openrouter", provider: "openrouter", model: "anthropic/claude-sonnet-4.6" });
  assert.deepStrictEqual(view.finitePrivate, { state: "unknown", reason: null });
  assert.deepStrictEqual(view.fallback, { state: "unknown", reason: null, model: null, extraEntries: 0 });
  // agentd reads the saved key itself; only what Hermes holds needs the helper.
  assert.equal(view.openrouter.state, "key_saved");
  assert.equal(view.openrouter.keySource, "agent");
  assert.equal(view.openrouter.keyHash, sha256("sk-or-v1-web-design-fixture-fake-key"));
  assert.equal(view.openrouter.hermesKey, "unknown");
  assert.equal(view.openrouter.otherPoolKeys, "unknown");

  assert.deepStrictEqual(fixture.commands, ["agent.owner.claim", "agent.connections.status"]);

  // With no saved key, agentd can't tell whether Hermes has one, so the route is unknown too.
  const noKey = fakeView({ agent: "pr1", unconfirmed: true });
  assert.equal(noKey.openrouter.state, "unknown");
  assert.equal(noKey.openrouter.hermesKey, "unknown");
  assert.deepStrictEqual(noKey.finitePrivate, { state: "unknown", reason: null });
  assert.equal(noKey.fallback.state, "unknown");

  // A legacy agent reports no facts at all, so the option changes nothing there.
  assert.deepStrictEqual(fakeView({ agent: "legacy", unconfirmed: true }), fakeView({ agent: "legacy" }));
});

test("set-agent names the agent, the saved route, and optionally that the agent couldn't confirm its facts", () => {
  assert.deepStrictEqual(parseInferenceChoice("pr1"), { agent: "pr1", saved: "finite_private", unconfirmed: false });
  assert.deepStrictEqual(parseInferenceChoice("pr1 openrouter"), { agent: "pr1", saved: "openrouter", unconfirmed: false });
  assert.deepStrictEqual(parseInferenceChoice("pr1 openrouter unconfirmed\n"), { agent: "pr1", saved: "openrouter", unconfirmed: true });
  for (const text of ["", "pr2", "pr1 anthropic", "pr1 openrouter unknown", "pr1 openrouter unconfirmed extra"]) {
    assert.equal(parseInferenceChoice(text), null, text);
  }
});

test("T-W21 (fake): FP ↔ OpenRouter through select runs the operation through its phases, then the saved route changes", async (t) => {
  const fixture = installFixture(t, { agent: "pr1" });

  // A PR1 agent has no connect command, so a pasted key goes through v1 with the model (§10.4).
  const paste = await post({ action: "inference", profile: "openrouter", apiKey: PASTED_KEY, model: "openai/gpt-5" });
  assert.equal(paste.status, 200);
  let view = inferenceView(paste.body);
  assert.deepStrictEqual(view.saved, { route: "openrouter", provider: "openrouter", model: "openai/gpt-5" });
  assert.equal(view.openrouter.state, "key_saved");
  assert.equal(view.openrouter.keySource, "agent");
  assert.equal(view.openrouter.keyHash, sha256(PASTED_KEY));
  assert.equal(paste.body.inference.profile, "openrouter");

  const toFinitePrivate = await post({ action: "inference_select", route: "finite_private" });
  assert.equal(toFinitePrivate.status, 200);
  assert.equal(toFinitePrivate.body.result.accepted, true);
  view = inferenceView(toFinitePrivate.body.status);
  assert.equal(view.operation?.id, toFinitePrivate.body.result.operation_id);
  assert.deepStrictEqual(
    [view.operation?.kind, view.operation?.route, view.operation?.state, view.operation?.phase],
    ["select", "finite_private", "running", "accepted"]
  );
  assert.equal(view.saved.route, "openrouter");
  assert.equal(pollDelayMs(view, true), 3_000);

  const busy = await post({ action: "inference_select", route: "openrouter", model: "openai/gpt-5" });
  assert.equal(busy.status, 502);
  assert.equal(busy.body.code, "operation_in_progress");
  assert.equal(busy.body.error, "Another connection change is still finishing. Try again in a moment.");

  assert.deepStrictEqual(await runOperation(fixture), ["config_written", "restarting", "verifying", "succeeded:verifying"]);
  ({ view } = await readStatus());
  assert.deepStrictEqual(view.saved, { route: "finite_private", provider: "custom", model: "glm-5-3-flash" });
  assert.equal(pollDelayMs(view, true), null);
  assert.equal(view.openrouter.state, "key_saved");

  const toOpenRouter = await post({ action: "inference_select", route: "openrouter", model: "openai/gpt-5" });
  assert.equal(toOpenRouter.body.result.accepted, true);
  assert.notEqual(toOpenRouter.body.result.operation_id, toFinitePrivate.body.result.operation_id);
  assert.deepStrictEqual(await runOperation(fixture), ["config_written", "restarting", "verifying", "succeeded:verifying"]);
  const { status } = await readStatus();
  assert.deepStrictEqual(inferenceView(status).saved, { route: "openrouter", provider: "openrouter", model: "openai/gpt-5" });
  assert.deepStrictEqual(
    [status.inference.profile, status.inference.provider, status.inference.model],
    ["openrouter", "openrouter", "openai/gpt-5"]
  );

  const again = await post({ action: "inference_select", route: "openrouter", model: "openai/gpt-5" });
  assert.deepStrictEqual(again.body.result, { changed: false });

  // The last result stays visible for 10 minutes, then status has no operation.
  fixture.tick(10 * 60_000);
  assert.equal((await readStatus()).view.operation, null);

  assert.deepStrictEqual(fixture.logs, []);
  assert.ok(fixture.replies.every((reply) => !reply.includes(PASTED_KEY)));
});

test("T-W23 (fake): a failed disconnect shows failed, blocks other changes, and Try again resumes the same operation", async (t) => {
  const fixture = installFixture(t, { agent: "pr1", saved: "openrouter" });
  fixture.fake.failNextOperation();
  const keyHash = (await readStatus()).view.openrouter.keyHash;
  assert.match(keyHash ?? "", /^[0-9a-f]{64}$/u);

  const first = await post({ action: "inference_disconnect", route: "openrouter" });
  assert.equal(first.body.result.accepted, true);
  const operationId = first.body.result.operation_id;
  assert.deepStrictEqual(await runOperation(fixture), [
    "login_cancelled", "route_switched", "credential_removed", "cleanup", "verifying", "failed:verifying",
  ]);
  let { view } = await readStatus();
  assert.deepStrictEqual(
    [view.operation?.id, view.operation?.kind, view.operation?.state, view.operation?.errorCode, view.operation?.attempts],
    [operationId, "disconnect", "failed", "verify_failed", 3]
  );
  assert.equal(pollDelayMs(view, true), null);
  assert.equal(view.saved.route, "finite_private");

  for (const payload of [
    { action: "inference_select", route: "finite_private" },
    { action: "inference", profile: "finite_private" },
  ]) {
    const refused = await post(payload);
    assert.equal(refused.status, 502);
    assert.equal(refused.body.code, "operation_in_progress");
  }

  const retry = await post({ action: "inference_disconnect", route: "openrouter" });
  assert.deepStrictEqual(retry.body.result, { accepted: true, operation_id: operationId });
  view = inferenceView(retry.body.status);
  assert.deepStrictEqual(
    [view.operation?.state, view.operation?.phase, view.operation?.errorCode, view.operation?.attempts],
    ["running", "verifying", null, 0]
  );
  assert.deepStrictEqual(await runOperation(fixture), ["succeeded:verifying"]);
  ({ view } = await readStatus());
  assert.equal(view.saved.route, "finite_private");
  assert.deepStrictEqual(view.openrouter, { state: "no_key", keySource: null, keyHash: null, hermesKey: "none", otherPoolKeys: "none" });

  const nothingLeft = await post({ action: "inference_disconnect", route: "openrouter" });
  assert.deepStrictEqual(nothingLeft.body.result, { changed: false });
});

test("a failed select: a restart failure restores the previous route, and the next change replaces the failed record", async (t) => {
  const fixture = installFixture(t, { agent: "pr1", saved: "openrouter" });
  fixture.fake.failNextOperation("supervisor_unavailable");
  await post({ action: "inference_select", route: "finite_private" });
  assert.deepStrictEqual(await runOperation(fixture), ["config_written", "restarting", "verifying", "failed:verifying"]);
  let { view } = await readStatus();
  assert.equal(view.operation?.errorCode, "supervisor_unavailable");
  assert.equal(view.saved.route, "openrouter");

  const retry = await post({ action: "inference_select", route: "finite_private" });
  assert.equal(retry.body.result.accepted, true);
  assert.equal(inferenceView(retry.body.status).operation?.state, "running");
  await runOperation(fixture);

  // A failed select whose write stuck is dropped by a select that turns out to be a no-op.
  fixture.fake.failNextOperation("verify_failed");
  await post({ action: "inference_select", route: "openrouter", model: "openai/gpt-5" });
  assert.deepStrictEqual((await runOperation(fixture)).at(-1), "failed:verifying");
  const noOp = await post({ action: "inference_select", route: "openrouter", model: "openai/gpt-5" });
  assert.deepStrictEqual(noOp.body.result, { changed: false });
  ({ view } = await readStatus());
  assert.equal(view.operation, null);
  assert.equal(view.saved.model, "openai/gpt-5");
});

test("a PR1 agent: PR2 and PR3 actions are gated by the dashboard, and the fake refuses them as agentd does", async (t) => {
  const fixture = installFixture(t, { agent: "pr1", saved: "openrouter" });
  const connect = await post({ action: "openrouter_connect_key", apiKey: PASTED_KEY });
  assert.equal(connect.status, 409);
  assert.equal(connect.body.code, "agent_update_required");
  const usage = await readJson("openrouter/usage");
  assert.equal(usage.status, 409);
  assert.ok(!fixture.commands.includes("agent.openrouter.connect"));

  for (const [command, schema] of [
    ["agent.openrouter.connect", "finite.agent.openrouter.connect.v1"],
    ["agent.openrouter.usage", "finite.agent.empty.request.v1"],
    ["agent.codex.login.start", "finite.agent.codex.login.start.v1"],
    ["agent.codex.models", "finite.agent.empty.request.v1"],
  ]) {
    const reply = fixture.fake.runtimeCommand({ command, schema, body: {} });
    assert.equal(reply.status, "failed");
    assert.equal(reply.error?.code, "unsupported_command");
  }
  const codexSelect = fixture.fake.runtimeCommand({
    command: "agent.inference.select", schema: "finite.agent.inference.select.v1", body: { route: "openai_codex", model: "gpt-5.5" },
  });
  assert.equal(codexSelect.error?.code, "invalid_payload");
  const wrongSchema = fixture.fake.runtimeCommand({
    command: "agent.inference.select", schema: "finite.agent.empty.request.v1", body: { route: "finite_private" },
  });
  assert.equal(wrongSchema.error?.code, "invalid_payload");
  const unknownField = fixture.fake.runtimeCommand({
    command: "agent.inference.disconnect", schema: "finite.agent.inference.disconnect.v1", body: { route: "openrouter", force: true },
  });
  assert.equal(unknownField.error?.code, "invalid_payload");
});

test("T-W22 (fake): a legacy agent sends today's status, keeps the v1 path, and refuses new commands", async (t) => {
  const fixture = installFixture(t, { agent: "legacy" });
  const raw = fixture.fake.runtimeCommand({ command: "agent.connections.status", schema: "finite.agent.empty.request.v1", body: {} });
  const body = raw.body as Record<string, Record<string, unknown>>;
  assert.deepStrictEqual(Object.keys(body).sort(), ["google", "inference", "telegram"]);
  assert.deepStrictEqual(body.inference, { profile: "finite_private", provider: "custom", model: "glm-5-3-flash" });

  const { view } = await readStatus();
  assert.equal(view.v2, false);
  assert.equal(view.saved.route, "finite_private");

  const paste = await post({ action: "inference", profile: "openrouter", apiKey: PASTED_KEY, model: "openai/gpt-5" });
  assert.equal(paste.status, 200);
  assert.deepStrictEqual(paste.body.inference, { profile: "openrouter", provider: "openrouter", model: "openai/gpt-5" });
  const back = await post({ action: "inference", profile: "finite_private" });
  assert.deepStrictEqual(back.body.inference, { profile: "finite_private", provider: "custom", model: "glm-5-3-flash" });

  const select = await post({ action: "inference_select", route: "finite_private" });
  assert.equal(select.status, 409);
  assert.equal(select.body.code, "agent_update_required");
  for (const [command, schema] of [
    ["agent.inference.select", "finite.agent.inference.select.v1"],
    ["agent.inference.disconnect", "finite.agent.inference.disconnect.v1"],
    ["agent.openrouter.connect", "finite.agent.openrouter.connect.v1"],
    ["agent.openrouter.usage", "finite.agent.empty.request.v1"],
    ["agent.codex.login.start", "finite.agent.codex.login.start.v1"],
    ["agent.codex.login.cancel", "finite.agent.codex.login.cancel.v1"],
    ["agent.codex.models", "finite.agent.empty.request.v1"],
  ]) {
    const reply = fixture.fake.runtimeCommand({ command, schema, body: {} });
    assert.deepStrictEqual([reply.status, reply.error?.code], ["failed", "unsupported_command"]);
  }
});

test("T-W22 (fake): a legacy agent with Codex saved still says finite_private, and the view reads ChatGPT", async (t) => {
  const fixture = installFixture(t, { agent: "legacy", saved: "openai_codex" });
  const raw = fixture.fake.runtimeCommand({ command: "agent.connections.status", schema: "finite.agent.empty.request.v1", body: {} });
  assert.deepStrictEqual((raw.body as { inference: unknown }).inference, {
    profile: "finite_private", provider: "openai-codex", model: "gpt-5.5",
  });
  const { view } = await readStatus();
  assert.equal(view.saved.route, "openai_codex");
  assert.equal(routeLabel(view.saved.route), "ChatGPT");
});

test("a full agent answers connect, usage, and Codex commands statefully", async (t) => {
  const fixture = installFixture(t, { agent: "full" });

  const stored = await post({ action: "openrouter_connect_key", apiKey: PASTED_KEY });
  assert.deepStrictEqual(stored.body.result, { changed: true, activated: false });
  assert.equal(inferenceView(stored.body.status).openrouter.keyHash, sha256(PASTED_KEY));
  assert.equal(inferenceView(stored.body.status).saved.route, "finite_private");

  for (const [key, code, message] of [
    ["sk-or-v1-fake-management-key", "credential_rejected", "That's an OpenRouter management key. Use an ordinary API key."],
    ["sk-or-v1-fake-exhausted-key", "key_allowance_exhausted", "This key has no remaining allowance. Raise its limit at openrouter.ai/keys, or use another key."],
    ["sk-or-v1-fake-rejected-key", "credential_rejected", "OpenRouter didn't accept this key."],
    ["sk-or-v1-fake-unreachable-key", "provider_unavailable", "Couldn't reach OpenRouter to check this key."],
  ]) {
    const refused = await post({ action: "openrouter_connect_key", apiKey: key });
    assert.deepStrictEqual([refused.status, refused.body.code, refused.body.error], [502, code, message]);
  }
  assert.equal((await readStatus()).view.openrouter.keyHash, sha256(PASTED_KEY));

  const activate = await post({ action: "openrouter_connect_key", apiKey: PASTED_KEY, activate: { model: "openai/gpt-5" } });
  assert.equal(activate.body.result.accepted, true);
  assert.equal(inferenceView(activate.body.status).operation?.kind, "activate");
  assert.deepStrictEqual((await runOperation(fixture)).at(-1), "succeeded:verifying");
  assert.equal((await readStatus()).view.saved.model, "openai/gpt-5");

  const usage = await readJson("openrouter/usage");
  assert.equal(usage.status, 200);
  assert.equal(usage.body.state, "ok");
  assert.equal(usage.body.key.key_hash, sha256(PASTED_KEY));
  assert.equal(usage.body.hermes_key, "saved_key");

  const models = await readJson("codex/models");
  assert.deepStrictEqual(models.body, { state: "unavailable", models: [], reason: "not_signed_in" });

  const login = await post({ action: "codex_login_start" });
  assert.equal(login.body.result.state, "pending");
  assert.equal(login.body.result.verification_uri, "https://auth.openai.com/codex/device");
  assert.equal(login.body.result.user_code, "FAKE-0000");
  let { view } = await readStatus();
  assert.equal(view.codex?.state, "not_signed_in");
  assert.equal(pollDelayMs(view, true), 3_000);
  fixture.tick(3 * PHASE_MS);
  assert.equal((await readStatus()).view.codex?.login?.state, "committing");
  fixture.tick();
  ({ view } = await readStatus());
  assert.deepStrictEqual([view.codex?.state, view.codex?.login?.state], ["signed_in", "approved"]);
  assert.equal(pollDelayMs(view, true), null);
  assert.deepStrictEqual((await readJson("codex/models")).body, { state: "live", models: ["gpt-5.5", "gpt-5.3-codex"], reason: null });

  const unknownModel = await post({ action: "inference_select", route: "openai_codex", model: "gpt-unknown" });
  assert.equal(unknownModel.body.code, "model_unavailable");
  await post({ action: "inference_select", route: "openai_codex", model: "gpt-5.5" });
  await runOperation(fixture);
  const { status } = await readStatus();
  assert.deepStrictEqual(inferenceView(status).saved, { route: "openai_codex", provider: "openai-codex", model: "gpt-5.5" });
  assert.equal(status.inference.profile, "finite_private");

  const disconnect = await post({ action: "inference_disconnect", route: "openai_codex" });
  assert.equal(disconnect.body.result.accepted, true);
  const blocked = await post({ action: "codex_login_start" });
  assert.deepStrictEqual([blocked.body.code, blocked.body.error], [
    "disconnect_in_progress",
    "ChatGPT is being removed from this agent. Wait for that to finish, or try the removal again.",
  ]);
  await runOperation(fixture);
  ({ view } = await readStatus());
  assert.deepStrictEqual([view.saved.route, view.codex?.state, view.codex?.login], ["finite_private", "not_signed_in", null]);

  const cancelStart = await post({ action: "codex_login_start" });
  const cancelled = await post({ action: "codex_login_cancel", attemptId: cancelStart.body.result.attempt_id });
  assert.equal(cancelled.body.result.state, "canceled");

  assert.deepStrictEqual(fixture.logs, []);
  assert.ok(fixture.replies.every((reply) => !reply.includes(PASTED_KEY)));
});

test("a full agent's OAuth connect is single-use per attempt", (t) => {
  const fixture = installFixture(t, { agent: "full" });
  const connect = (attempt: string, code: string) =>
    fixture.fake.runtimeCommand({
      command: "agent.openrouter.connect",
      schema: "finite.agent.openrouter.connect.v1",
      body: {
        credential: { kind: "oauth_code", code, code_verifier: "v".repeat(43), attempt_id: `ora_${attempt.repeat(32)}` },
        activate: null,
      },
    });
  const first = connect("a", "fake-code");
  assert.deepStrictEqual(first.body, { changed: true, activated: false });
  assert.deepStrictEqual(connect("a", "fake-code").body, first.body);
  assert.equal(connect("b", "fake-rejected-code").error?.code, "credential_rejected");
  assert.equal(connect("b", "fake-rejected-code").error?.code, "attempt_not_found");
});

test("FC_DESIGN_RUNTIME_COMMANDS_URL accepts only a loopback origin", () => {
  assert.equal(runtimeCommandsForwardUrl(undefined), null);
  assert.equal(runtimeCommandsForwardUrl(""), null);
  assert.equal(runtimeCommandsForwardUrl("http://127.0.0.1:18080"), "http://127.0.0.1:18080/v1/app/runtime-commands");
  assert.equal(runtimeCommandsForwardUrl("http://localhost:18080/"), "http://localhost:18080/v1/app/runtime-commands");
  for (const value of [
    "https://127.0.0.1:18080",
    "http://127.0.0.1",
    "http://127.0.0.1:0",
    "http://127.0.0.1:65536",
    "http://127.0.0.1:18080/v1/app/runtime-commands",
    "http://127.0.0.1:18080?x=1",
    "http://user:pass@127.0.0.1:18080",
    "http://127.0.0.1.example.com:18080",
    "http://localhost.:18080",
    "http://127.1:18080",
    "http://[::1]:18080",
    "http://10.0.0.5:18080",
    "https://hwd.finite.example",
    " http://127.0.0.1:18080",
  ]) {
    assert.throws(() => runtimeCommandsForwardUrl(value), /must be http:\/\/127\.0\.0\.1:<port> or http:\/\/localhost:<port>/u, value);
  }
});

test("forwarding passes the runtime command to the local harness and its reply back unchanged", async (t) => {
  const received: Array<{ method?: string; url?: string; body: string; authorization?: string; user?: string }> = [];
  const harness = http.createServer(async (request, response) => {
    const chunks: Buffer[] = [];
    for await (const chunk of request) chunks.push(chunk as Buffer);
    received.push({
      method: request.method,
      url: request.url,
      body: Buffer.concat(chunks).toString("utf8"),
      authorization: request.headers.authorization,
      user: request.headers["x-finite-workos-user-id"] as string | undefined,
    });
    response.writeHead(200, { "content-type": "application/json" });
    response.end(JSON.stringify({ request_id: "harness-1", status: "failed", body: null, error: { code: "operation_in_progress", message: "busy" } }));
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

  const command = JSON.stringify({ command: "agent.inference.select", schema: "finite.agent.inference.select.v1", body: { route: "finite_private", model: null } });
  const response = await fetch(`http://127.0.0.1:${(fixture.address() as { port: number }).port}/v1/app/runtime-commands`, {
    method: "POST",
    headers: { authorization: "Bearer web-design-hosted-device-token", "x-finite-workos-user-id": "user_web_design", "content-type": "application/json" },
    body: command,
  });
  assert.equal(response.status, 200);
  assert.deepStrictEqual(await response.json(), {
    request_id: "harness-1", status: "failed", body: null, error: { code: "operation_in_progress", message: "busy" },
  });
  assert.deepStrictEqual(received, [{
    method: "POST",
    url: "/v1/app/runtime-commands",
    body: command,
    authorization: "Bearer web-design-hosted-device-token",
    user: "user_web_design",
  }]);
});

test("the fixture refuses to start when FC_DESIGN_RUNTIME_COMMANDS_URL is not local", { timeout: 180_000 }, async () => {
  const child = spawn(process.execPath, ["--import", "tsx", "scripts/web-design-fixture.ts", "serve"], {
    cwd: path.resolve(__dirname, "../.."),
    env: { ...process.env, FC_DESIGN_RUNTIME_COMMANDS_URL: "https://user:not-a-real-token@hwd.example", FC_WEB_DESIGN_PORT: "1" },
    stdio: ["ignore", "pipe", "pipe"],
  });
  let output = "";
  child.stdout.on("data", (chunk) => (output += chunk));
  child.stderr.on("data", (chunk) => (output += chunk));
  const [code] = await once(child, "exit");
  assert.equal(code, 1);
  assert.match(output, /FC_DESIGN_RUNTIME_COMMANDS_URL must be http:\/\/127\.0\.0\.1:<port> or http:\/\/localhost:<port>/u);
  assert.doesNotMatch(output, /not-a-real-token|Real dashboard UI/u);
});
