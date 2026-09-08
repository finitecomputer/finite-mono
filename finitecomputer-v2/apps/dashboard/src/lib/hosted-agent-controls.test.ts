import assert from "node:assert/strict";
import test from "node:test";

import {
  HostedAgentControlError,
  agentOwnerClaimCommand,
  parseAgentConnectionAction,
  parseSimplexStatus,
} from "@/lib/hosted-agent-controls";

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
