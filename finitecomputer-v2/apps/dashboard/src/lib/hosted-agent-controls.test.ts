import assert from "node:assert/strict";
import test from "node:test";

import {
  HostedAgentControlError,
  agentOwnerClaimCommand,
  parseAgentConnectionAction,
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
