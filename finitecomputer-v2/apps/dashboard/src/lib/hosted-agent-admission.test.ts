import assert from "node:assert/strict";
import test from "node:test";

import {
  CHAT_ADMISSION_COMMAND,
  CHAT_ADMISSION_SCHEMA,
  CHAT_ADMISSION_WAIT_MILLIS,
  chatAdmissionCommand,
  dispatchChatAdmissionCommand,
  isResultLessAdmissionTimeout,
  parseChatAdmissionAction,
} from "@/lib/hosted-agent-admission";
import {
  HostedAgentControlError,
  type AgentCommandContext,
} from "@/lib/hosted-agent-controls";
import { HostedDeviceRequestError } from "@/lib/hosted-web-device";

const verifiedAccount = {
  email: "paul@finite.vip",
  workosUserId: "user_paul",
  emailVerified: true,
  source: "workos" as const,
};

const admissionContext: AgentCommandContext = {
  account: verifiedAccount,
  config: { baseUrl: "https://device.internal", apiToken: "internal-token" },
  roomId: "room-a",
  targetAccountId: "agent-a",
};

const strangerAccountId = "cd".repeat(32);

test("chat admission commands ride the owner runtime-command channel unchanged", () => {
  assert.deepEqual(
    chatAdmissionCommand("room-1", "agent-account-1", {
      action: "revoke",
      accountId: strangerAccountId,
    }),
    {
      room_id: "room-1",
      target_account_id: "agent-account-1",
      command: CHAT_ADMISSION_COMMAND,
      schema: CHAT_ADMISSION_SCHEMA,
      body: { action: "revoke", account_id: strangerAccountId },
      wait_millis: CHAT_ADMISSION_WAIT_MILLIS,
    }
  );
  assert.equal(CHAT_ADMISSION_COMMAND, "chat.admission");
  assert.equal(CHAT_ADMISSION_SCHEMA, "finite.chat.admission.v1");
  // The sidecar never posts a result for chat.admission, so the wait stays at
  // the shortest window the hosted device accepts instead of the 45s owner
  // claim pattern.
  assert.equal(CHAT_ADMISSION_WAIT_MILLIS, 1_000);
});

test("chat admission actions are grant or revoke over a normalized account id", () => {
  assert.deepEqual(
    parseChatAdmissionAction({ action: "grant", accountId: ` ${strangerAccountId.toUpperCase()} ` }),
    { action: "grant", accountId: strangerAccountId }
  );
  assert.deepEqual(
    parseChatAdmissionAction({ action: "revoke", accountId: strangerAccountId }),
    { action: "revoke", accountId: strangerAccountId }
  );
  assert.throws(
    () => parseChatAdmissionAction({ action: "invite", accountId: strangerAccountId }),
    /Choose grant or revoke/u
  );
  assert.throws(
    () => parseChatAdmissionAction({ action: "grant", accountId: "npub1not-hex" }),
    /64 hexadecimal/u
  );
  assert.throws(
    () => parseChatAdmissionAction({ action: "grant", accountId: "cd".repeat(63) }),
    /64 hexadecimal/u
  );
  assert.throws(() => parseChatAdmissionAction(null), HostedAgentControlError);
});

test("a result-less admission timeout is the expected silent apply", async (context) => {
  const originalFetch = global.fetch;
  context.after(() => {
    global.fetch = originalFetch;
  });
  let observedUrl = "";
  let observedBody = "";
  global.fetch = (async (input, init) => {
    observedUrl = String(input);
    observedBody = String(init?.body);
    return Response.json(
      { error: "The agent did not respond in time. Try again." },
      { status: 500 }
    );
  }) as typeof fetch;

  const result = await dispatchChatAdmissionCommand(admissionContext, {
    action: "revoke",
    accountId: strangerAccountId,
  });

  assert.equal(observedUrl, "https://device.internal/v1/app/runtime-commands");
  assert.deepEqual(JSON.parse(observedBody), {
    room_id: "room-a",
    target_account_id: "agent-a",
    command: "chat.admission",
    schema: "finite.chat.admission.v1",
    body: { action: "revoke", account_id: strangerAccountId },
    wait_millis: 1_000,
  });
  assert.deepEqual(result, { status: "sent" });
});

test("a terminal admission result reports an explicit apply", async (context) => {
  const originalFetch = global.fetch;
  context.after(() => {
    global.fetch = originalFetch;
  });
  global.fetch = (async () =>
    Response.json({
      request_id: "admission-a",
      status: "succeeded",
      body: null,
      error: null,
    })) as typeof fetch;

  assert.deepEqual(
    await dispatchChatAdmissionCommand(admissionContext, {
      action: "grant",
      accountId: strangerAccountId,
    }),
    { status: "applied" }
  );
});

test("a refused admission result surfaces the agent's error", async (context) => {
  const originalFetch = global.fetch;
  context.after(() => {
    global.fetch = originalFetch;
  });
  global.fetch = (async () =>
    Response.json({
      request_id: "admission-a",
      status: "failed",
      body: null,
      error: { code: "unauthorized", message: "sender is not on the Welcome allowlist" },
    })) as typeof fetch;

  await assert.rejects(
    dispatchChatAdmissionCommand(admissionContext, {
      action: "revoke",
      accountId: strangerAccountId,
    }),
    (error: unknown) => {
      assert.ok(error instanceof HostedAgentControlError);
      assert.equal(error.status, 403);
      assert.match(error.message, /Welcome allowlist/u);
      return true;
    }
  );
});

test("admission transport failures other than the result-less timeout propagate", async (context) => {
  const originalFetch = global.fetch;
  context.after(() => {
    global.fetch = originalFetch;
  });
  global.fetch = (async () =>
    Response.json({ error: "hosted device is down" }, { status: 502 })
  ) as typeof fetch;
  await assert.rejects(
    dispatchChatAdmissionCommand(admissionContext, {
      action: "grant",
      accountId: strangerAccountId,
    }),
    (error: unknown) => {
      assert.ok(error instanceof HostedDeviceRequestError);
      assert.equal(error.status, 502);
      return true;
    }
  );
  assert.equal(
    isResultLessAdmissionTimeout(
      new HostedDeviceRequestError("The agent did not respond in time. Try again.", 500)
    ),
    true
  );
  assert.equal(
    isResultLessAdmissionTimeout(new HostedDeviceRequestError("other task failure", 500)),
    false
  );
});
