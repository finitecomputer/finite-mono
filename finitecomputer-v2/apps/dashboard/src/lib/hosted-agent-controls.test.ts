import assert from "node:assert/strict";
import test from "node:test";

import {
  HostedAgentControlError,
  parseAgentConnectionAction,
} from "@/lib/hosted-agent-controls";

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
