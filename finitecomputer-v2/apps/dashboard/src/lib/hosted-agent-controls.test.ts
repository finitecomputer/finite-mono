import assert from "node:assert/strict";
import test from "node:test";

import {
  AGENT_ACCOUNT_ID,
  BRIDGE_URL,
  DEVICE_URL,
  directInfo,
  directWorld,
  fixtureAccess,
  fixtureBridge,
  type RecordedRequest,
} from "@/lib/direct-chat-fixture";
import {
  HostedAgentControlError,
  agentCommandContextFor,
  agentOwnerClaimCommand,
  loadAgentConnectionsWithContext,
  parseAgentConnectionAction,
  parseSimplexStatus,
} from "@/lib/hosted-agent-controls";

const commandTarget = {
  account: fixtureAccess.account,
  config: fixtureAccess.hosted,
  runtimeId: fixtureAccess.runtimeId,
  projectId: fixtureAccess.projectId,
  primaryUrl: null,
  displayName: "Moss",
};

const connectionsStatus = {
  inference: { profile: "finite_private", provider: "finite", model: "finite-default" },
  telegram: { connected: false, pending: [], approved: [] },
  google: { connected: false },
};

const commandOf = (request: RecordedRequest) =>
  JSON.parse(new TextDecoder().decode(request.body!)) as { command: string; room_id: string; target_account_id: string };

test("Connections commands reach a direct agent through its own chat bridge", async (context) => {
  const world = directWorld(context, {
    "POST /v1/app/runtime-commands": (request) => Response.json({
      request_id: "request-1",
      status: "succeeded",
      body: commandOf(request).command === "agent.connections.status" ? connectionsStatus : {},
    }),
  });
  const commands = await agentCommandContextFor(commandTarget, fixtureBridge().bridge);
  assert.equal(commands.roomId, "room-1");
  assert.equal(commands.targetAccountId, AGENT_ACCOUNT_ID);

  const status = await loadAgentConnectionsWithContext(commands);
  assert.equal(status.inference.profile, "finite_private");
  const sent = world.agentRequests();
  assert(sent.every((request) => request.url === `${BRIDGE_URL}/v1/app/runtime-commands`));
  assert.deepEqual(sent.map((request) => commandOf(request).command), [
    "agent.owner.claim",
    "agent.connections.status",
  ]);
  assert(sent.every((request) => commandOf(request).room_id === "room-1"));
  assert(sent.every((request) => commandOf(request).target_account_id === AGENT_ACCOUNT_ID));
  assert.deepEqual(world.hostedAppRequests(), [], "no hosted-device binding for a direct agent");
});

test("Connections fail closed with Connections copy when the chat target is unavailable or ambiguous", async (context) => {
  directWorld(context, {}, { info: () => new Response(null, { status: 503 }) });
  await assert.rejects(
    agentCommandContextFor(commandTarget, fixtureBridge().bridge),
    (error: unknown) =>
      error instanceof HostedAgentControlError &&
      error.status === 503 &&
      error.message === "Connections are unavailable right now."
  );

  directWorld(context, {}, {
    info: () => Response.json(directInfo({ rooms: ["room-a", "room-b"], canonical_room_id: null })),
  });
  await assert.rejects(
    agentCommandContextFor(commandTarget, fixtureBridge().bridge),
    (error: unknown) => error instanceof HostedAgentControlError && error.status === 409
  );
});

test("Connections for a legacy agent stay on the hosted-device binding", async (context) => {
  const world = directWorld(context, {}, {
    info: () => Response.json({ detail: "Not Found" }, { status: 404 }),
    fallback: (request) => {
      if (request.url === `${DEVICE_URL}/v1/app/agent-bindings/open`) {
        return Response.json({
          hosted_agent_binding: { canonical_room_id: "room-legacy", agent_account_id: "agent-legacy" },
        });
      }
      if (request.url === `${DEVICE_URL}/v1/app/runtime-commands`) {
        return Response.json({ request_id: "r1", status: "succeeded", body: connectionsStatus });
      }
      return new Response(null, { status: 404 });
    },
  });
  const commands = await agentCommandContextFor(commandTarget, fixtureBridge().bridge);
  assert.deepEqual([commands.roomId, commands.targetAccountId], ["room-legacy", "agent-legacy"]);
  await loadAgentConnectionsWithContext(commands);
  assert.deepEqual(
    world.hostedAppRequests().map((request) => new URL(request.url).pathname),
    ["/v1/app/agent-bindings/open", "/v1/app/runtime-commands", "/v1/app/runtime-commands"]
  );
  assert.deepEqual(world.agentRequests(), []);
});

test("Connections reuses the durable successful owner claim", () => {
  assert.deepEqual(agentOwnerClaimCommand("room-1", "agent-account-1"), {
    room_id: "room-1",
    target_account_id: "agent-account-1",
    command: "agent.owner.claim",
    resource_key: "agent.connections",
    schema: "finite.agent.empty.request.v1",
    body: {},
    reuse_succeeded_owner_claim: true,
    wait_millis: 45_000,
  });
});

test("connection actions expose only the product-scoped command surface", () => {
  assert.deepEqual(parseAgentConnectionAction({ action: "status" }), { action: "status" });
  assert.deepEqual(
    parseAgentConnectionAction({
      action: "inference",
      profile: "openrouter",
      apiKey: "key-value",
      model: "anthropic/claude-sonnet-4.6",
    }),
    {
      action: "inference",
      profile: "openrouter",
      apiKey: "key-value",
      model: "anthropic/claude-sonnet-4.6",
    }
  );
  assert.deepEqual(
    parseAgentConnectionAction({ action: "telegram_approve", code: "ABCD2345" }),
    { action: "telegram_approve", code: "ABCD2345" }
  );
  assert.throws(
    () => parseAgentConnectionAction({ action: "run", command: "rm", args: ["-rf"] }),
    HostedAgentControlError
  );
});

test("connection actions reject unknown inference and oversized secrets", () => {
  assert.throws(
    () => parseAgentConnectionAction({ action: "inference", profile: "anything" }),
    /Choose Finite Private or OpenRouter/u
  );
  assert.throws(
    () =>
      parseAgentConnectionAction({
        action: "telegram_connect",
        token: "x".repeat(257),
      }),
    HostedAgentControlError
  );
});

test("SimpleX exposes pairing actions without arbitrary daemon commands", () => {
  for (const action of ["simplex_connect", "simplex_reset"]) {
    assert.deepEqual(parseAgentConnectionAction({ action }), { action });
  }
  assert.throws(() => parseAgentConnectionAction({ action: "simplex_approve", code: "ABCD2345" }));
  assert.throws(() => parseAgentConnectionAction({ action: "simplex_disconnect" }));
  assert.throws(() => parseAgentConnectionAction({ action: "simplex_command", command: "/sql" }));
});

test("SimpleX rejects executable links and malformed QR matrices", () => {
  const status = { enabled: true, ready: true, address: "https://smp.example/a#synthetic", qr: ["10", "01"], approved: [] };
  assert.equal(parseSimplexStatus(status).address, status.address);
  assert.throws(() => parseSimplexStatus({ ...status, address: "javascript:alert(1)" }));
  assert.throws(() => parseSimplexStatus({ ...status, qr: ["10", "1"] }));
  assert.throws(() => parseSimplexStatus({ ...status, qr: ["<svg>"] }));
});


test("SimpleX pending requests retain exact IDs and reject invalid metadata", () => {
  const request = { request_id: "abcdef0123456789", user_id: "3", name: "Owner", age_minutes: 2 };
  const status = { enabled: true, ready: true, qr: [], approved: [], pending: [request] };
  assert.deepEqual(parseSimplexStatus(status).pending, [request]);
  assert.equal(parseSimplexStatus({ ...status, pending: undefined }).pending, undefined);
  assert.deepEqual(parseAgentConnectionAction({ action: "simplex_approve_request", request_id: request.request_id }), { action: "simplex_approve_request", request_id: request.request_id });
  assert.throws(() => parseAgentConnectionAction({ action: "simplex_approve_request", request_id: "Owner" }));
  assert.throws(() => parseSimplexStatus({ ...status, pending: [{ ...request, request_id: "Owner" }] }));
  assert.throws(() => parseSimplexStatus({ ...status, pending: [{ ...request, age_minutes: -1 }] }));
});
