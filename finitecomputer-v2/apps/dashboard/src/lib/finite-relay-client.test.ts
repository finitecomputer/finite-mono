import assert from "node:assert/strict";
import test from "node:test";

import {
  callMachineRelay,
  callMachineChat,
  conversationRelayScope,
  createMachineChatConversation,
  fetchMachineChatConversations,
  fetchMachineChatMessages,
  fetchMachineChatStream,
  parseRelayResponseText,
  relayEndpointForSourceHost,
  relayStatusSnapshotValue,
  runtimeRelayScope,
  sendMachineChatMessage,
  updateMachineChatConversation,
} from "./finite-relay-client";

test("relay parser keeps plain text error bodies readable", () => {
  assert.throws(
    () => parseRelayResponseText("Failed to buffer request body", false, 413),
    /Failed to buffer request body \(413\)/
  );
});

test("relay parser rejects non-json success bodies", () => {
  assert.throws(
    () => parseRelayResponseText("not json", true, 200),
    /relay service returned non-JSON response \(200\)/
  );
});

test("relay parser returns json response bodies", () => {
  assert.deepEqual(parseRelayResponseText('{"ok":true}', true, 200), { ok: true });
});

test("relay endpoint map selects imported project relay by source host", () => {
  const endpoint = relayEndpointForSourceHost(" Smoke ", {
    FC_RELAY_HOST_ENDPOINTS_JSON: JSON.stringify({
      smoke: {
        url: "https://relay.smoke.finite.computer",
        adminToken: "smoke-token",
      },
    }),
  });

  assert.deepEqual(endpoint, {
    baseUrl: "https://relay.smoke.finite.computer",
    adminToken: "smoke-token",
  });
});

test("relay endpoint map returns null when host is not mapped", () => {
  assert.equal(
    relayEndpointForSourceHost("box1", {
      FC_RELAY_HOST_ENDPOINTS_JSON: JSON.stringify({
        smoke: {
          url: "https://relay.smoke.finite.computer",
          adminToken: "smoke-token",
        },
      }),
    }),
    null
  );
});

test("relay endpoint map fails closed for malformed host entries", () => {
  assert.throws(
    () =>
      relayEndpointForSourceHost("smoke", {
        FC_RELAY_HOST_ENDPOINTS_JSON: JSON.stringify({
          smoke: {
            url: "https://relay.smoke.finite.computer",
          },
        }),
      }),
    /requires url and adminToken/
  );
});

test("relay status snapshot helper accepts only fresh matching schemas", () => {
  const fresh = relayStatusSnapshotValue(
    {
      ok: true,
      machineId: "machine-1",
      stateKey: "runtime.inference.status",
      schema: "finitecomputer.runtime.inference.status.v1",
      revision: 1,
      status: { configured: true },
      observedAt: "2026-05-21T00:00:00Z",
      expiresAt: "2026-05-21T00:02:00Z",
    },
    "finitecomputer.runtime.inference.status.v1",
    Date.parse("2026-05-21T00:01:00Z")
  );
  assert.deepEqual(fresh, { configured: true });

  const expired = relayStatusSnapshotValue(
    {
      ok: true,
      machineId: "machine-1",
      stateKey: "runtime.inference.status",
      schema: "finitecomputer.runtime.inference.status.v1",
      revision: 1,
      status: { configured: true },
      observedAt: "2026-05-21T00:00:00Z",
      expiresAt: "2026-05-21T00:02:00Z",
    },
    "finitecomputer.runtime.inference.status.v1",
    Date.parse("2026-05-21T00:02:01Z")
  );
  assert.equal(expired, null);
});

test("relay status snapshot helper surfaces runtime errors without command fallback", () => {
  assert.throws(
    () => relayStatusSnapshotValue(
      {
        ok: false,
        machineId: "machine-1",
        stateKey: "runtime.inference.status",
        schema: "finitecomputer.runtime.inference.status.v1",
        revision: 1,
        error: "Hermes config is unreadable",
        observedAt: "2026-05-21T00:00:00Z",
        expiresAt: "2026-05-21T00:02:00Z",
      },
      "finitecomputer.runtime.inference.status.v1",
      Date.parse("2026-05-21T00:01:00Z")
    ),
    /Hermes config is unreadable/
  );
});

test("chat message page helper reads the relay finite chat log endpoint", async () => {
  const previousFetch = globalThis.fetch;
  const previousRelayUrl = process.env.FC_RELAY_URL;
  const previousRelayToken = process.env.FC_RELAY_ADMIN_TOKEN;
  let requested: URL | null = null;

  process.env.FC_RELAY_URL = "http://127.0.0.1:4100";
  process.env.FC_RELAY_ADMIN_TOKEN = "admin-token";
  globalThis.fetch = (async (input: RequestInfo | URL) => {
    requested = new URL(String(input));
    return new Response(JSON.stringify({ messages: [], has_more: false, next_before: null }), {
      status: 200,
    });
  }) as typeof fetch;

  try {
    const page = await fetchMachineChatMessages("smoke-finite", "thread-1", {
      bridgeAccountId: "hosted-web-user-1234",
      bridgeDeviceId: "dashboard-bridge-v1",
      limit: 25,
      before: "2026-05-22T00:00:00Z",
    });
    assert.deepEqual(page, { messages: [], has_more: false, next_before: null });
    const requestedUrl = requested as unknown as URL;
    assert.equal(
      requestedUrl.pathname,
      "/api/finite/v1/machines/smoke-finite/chat/conversations/thread-1/messages"
    );
    assert.equal(requestedUrl.searchParams.get("bridgeAccountId"), "hosted-web-user-1234");
    assert.equal(requestedUrl.searchParams.get("bridgeDeviceId"), "dashboard-bridge-v1");
    assert.equal(requestedUrl.searchParams.get("projectAgentId"), null);
    assert.equal(requestedUrl.searchParams.get("limit"), "25");
    assert.equal(requestedUrl.searchParams.get("before"), "2026-05-22T00:00:00Z");
  } finally {
    globalThis.fetch = previousFetch;
    if (previousRelayUrl === undefined) {
      delete process.env.FC_RELAY_URL;
    } else {
      process.env.FC_RELAY_URL = previousRelayUrl;
    }
    if (previousRelayToken === undefined) {
      delete process.env.FC_RELAY_ADMIN_TOKEN;
    } else {
      process.env.FC_RELAY_ADMIN_TOKEN = previousRelayToken;
    }
  }
});

test("chat conversation helper reads relay finite chat conversation projection", async () => {
  const previousFetch = globalThis.fetch;
  const previousRelayUrl = process.env.FC_RELAY_URL;
  const previousRelayToken = process.env.FC_RELAY_ADMIN_TOKEN;
  let requested: URL | null = null;

  process.env.FC_RELAY_URL = "http://127.0.0.1:4100";
  process.env.FC_RELAY_ADMIN_TOKEN = "admin-token";
  globalThis.fetch = (async (input: RequestInfo | URL) => {
    requested = new URL(String(input));
    return new Response(JSON.stringify([{ id: "thread-1" }]), {
      status: 200,
    });
  }) as typeof fetch;

  try {
    const threads = await fetchMachineChatConversations("smoke-finite", {
      bridgeAccountId: "hosted-web-user-1234",
      bridgeDeviceId: "dashboard-bridge-v1",
    });
    assert.deepEqual(threads, [{ id: "thread-1" }]);
    const requestedUrl = requested as unknown as URL;
    assert.equal(
      requestedUrl.pathname,
      "/api/finite/v1/machines/smoke-finite/chat/conversations"
    );
    assert.equal(requestedUrl.searchParams.get("bridgeAccountId"), "hosted-web-user-1234");
    assert.equal(requestedUrl.searchParams.get("bridgeDeviceId"), "dashboard-bridge-v1");
  } finally {
    globalThis.fetch = previousFetch;
    if (previousRelayUrl === undefined) {
      delete process.env.FC_RELAY_URL;
    } else {
      process.env.FC_RELAY_URL = previousRelayUrl;
    }
    if (previousRelayToken === undefined) {
      delete process.env.FC_RELAY_ADMIN_TOKEN;
    } else {
      process.env.FC_RELAY_ADMIN_TOKEN = previousRelayToken;
    }
  }
});

test("chat conversation create helper appends through finitechat relay conversations endpoint", async () => {
  const previousFetch = globalThis.fetch;
  const previousRelayUrl = process.env.FC_RELAY_URL;
  const previousRelayToken = process.env.FC_RELAY_ADMIN_TOKEN;
  const requests: Array<{ url: URL; body: unknown }> = [];

  process.env.FC_RELAY_URL = "http://127.0.0.1:4100";
  process.env.FC_RELAY_ADMIN_TOKEN = "admin-token";
  globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = new URL(String(input));
    requests.push({ url, body: init?.body ? JSON.parse(String(init.body)) : null });
    return new Response(JSON.stringify({ id: "topic-1", title: "New chat" }), {
      status: 200,
    });
  }) as typeof fetch;

  try {
    const thread = await createMachineChatConversation(
      "smoke-finite",
      {
        bridgeAccountId: "hosted-web-user-1234",
        bridgeDeviceId: "dashboard-bridge-v1",
      },
      {
        projectAgentId: "agent_smoke-finite",
        conversationId: "topic-1",
        title: " New chat ",
      }
    );
    assert.deepEqual(thread, { id: "topic-1", title: "New chat" });
    assert.equal(
      requests[0]?.url.pathname,
      "/api/finite/v1/machines/smoke-finite/chat/conversations"
    );
    assert.deepEqual(requests[0]?.body, {
      bridge: {
        bridgeAccountId: "hosted-web-user-1234",
        bridgeDeviceId: "dashboard-bridge-v1",
      },
      projectAgentId: "agent_smoke-finite",
      conversationId: "topic-1",
      title: "New chat",
    });
  } finally {
    globalThis.fetch = previousFetch;
    if (previousRelayUrl === undefined) {
      delete process.env.FC_RELAY_URL;
    } else {
      process.env.FC_RELAY_URL = previousRelayUrl;
    }
    if (previousRelayToken === undefined) {
      delete process.env.FC_RELAY_ADMIN_TOKEN;
    } else {
      process.env.FC_RELAY_ADMIN_TOKEN = previousRelayToken;
    }
  }
});

test("chat conversation update helper appends finitechat metadata update", async () => {
  const previousFetch = globalThis.fetch;
  const previousRelayUrl = process.env.FC_RELAY_URL;
  const previousRelayToken = process.env.FC_RELAY_ADMIN_TOKEN;
  const requests: Array<{ url: URL; body: unknown; method?: string }> = [];

  process.env.FC_RELAY_URL = "http://127.0.0.1:4100";
  process.env.FC_RELAY_ADMIN_TOKEN = "admin-token";
  globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = new URL(String(input));
    requests.push({
      url,
      body: init?.body ? JSON.parse(String(init.body)) : null,
      method: init?.method,
    });
    return new Response(JSON.stringify({ id: "topic-1", title: "Deploy runbook" }), {
      status: 200,
    });
  }) as typeof fetch;

  try {
    const thread = await updateMachineChatConversation(
      "smoke-finite",
      "topic-1",
      {
        bridgeAccountId: "hosted-web-user-1234",
        bridgeDeviceId: "dashboard-bridge-v1",
      },
      {
        projectAgentId: "agent_smoke-finite",
        title: " Deploy runbook ",
      }
    );
    assert.deepEqual(thread, { id: "topic-1", title: "Deploy runbook" });
    assert.equal(requests[0]?.method, "PUT");
    assert.equal(
      requests[0]?.url.pathname,
      "/api/finite/v1/machines/smoke-finite/chat/conversations/topic-1"
    );
    assert.deepEqual(requests[0]?.body, {
      bridge: {
        bridgeAccountId: "hosted-web-user-1234",
        bridgeDeviceId: "dashboard-bridge-v1",
      },
      projectAgentId: "agent_smoke-finite",
      title: "Deploy runbook",
    });
  } finally {
    globalThis.fetch = previousFetch;
    if (previousRelayUrl === undefined) {
      delete process.env.FC_RELAY_URL;
    } else {
      process.env.FC_RELAY_URL = previousRelayUrl;
    }
    if (previousRelayToken === undefined) {
      delete process.env.FC_RELAY_ADMIN_TOKEN;
    } else {
      process.env.FC_RELAY_ADMIN_TOKEN = previousRelayToken;
    }
  }
});

test("chat stream helper carries hosted bridge scope", async () => {
  const previousFetch = globalThis.fetch;
  const previousRelayUrl = process.env.FC_RELAY_URL;
  const previousRelayToken = process.env.FC_RELAY_ADMIN_TOKEN;
  let requested: URL | null = null;
  let requestedLastEventId: string | null = null;

  process.env.FC_RELAY_URL = "http://127.0.0.1:4100";
  process.env.FC_RELAY_ADMIN_TOKEN = "admin-token";
  globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
    requested = new URL(String(input));
    requestedLastEventId = new Headers(init?.headers).get("last-event-id");
    return new Response("event: chat.empty\ndata: {}\n\n", {
      status: 200,
      headers: { "content-type": "text/event-stream" },
    });
  }) as typeof fetch;

  try {
    const response = await fetchMachineChatStream(
      "smoke-finite",
      {
        bridgeAccountId: "hosted-web-user-1234",
        bridgeDeviceId: "dashboard-bridge-v1",
      },
      "v1.resume-cursor"
    );
    assert.equal(response.ok, true);
    const requestedUrl = requested as unknown as URL;
    assert.equal(
      requestedUrl.pathname,
      "/api/finite/v1/machines/smoke-finite/chat/stream"
    );
    assert.equal(requestedUrl.searchParams.get("bridgeAccountId"), "hosted-web-user-1234");
    assert.equal(requestedUrl.searchParams.get("bridgeDeviceId"), "dashboard-bridge-v1");
    assert.equal(requestedLastEventId, "v1.resume-cursor");
  } finally {
    globalThis.fetch = previousFetch;
    if (previousRelayUrl === undefined) {
      delete process.env.FC_RELAY_URL;
    } else {
      process.env.FC_RELAY_URL = previousRelayUrl;
    }
    if (previousRelayToken === undefined) {
      delete process.env.FC_RELAY_ADMIN_TOKEN;
    } else {
      process.env.FC_RELAY_ADMIN_TOKEN = previousRelayToken;
    }
  }
});

test("chat command helper sends hosted bridge scope on mutating commands", async () => {
  const previousFetch = globalThis.fetch;
  const previousRelayUrl = process.env.FC_RELAY_URL;
  const previousRelayToken = process.env.FC_RELAY_ADMIN_TOKEN;
  const requests: Array<{ url: URL; body: unknown }> = [];

  process.env.FC_RELAY_URL = "http://127.0.0.1:4100";
  process.env.FC_RELAY_ADMIN_TOKEN = "admin-token";
  globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = new URL(String(input));
    requests.push({ url, body: init?.body ? JSON.parse(String(init.body)) : null });
    if (url.pathname.endsWith("/events")) {
      return new Response(JSON.stringify({ id: "event-1" }), { status: 200 });
    }
    return new Response(JSON.stringify({ ok: true, output: { id: "msg-1" } }), {
      status: 200,
    });
  }) as typeof fetch;

  try {
    const result = await callMachineChat(
      "smoke-finite",
      "chat.send_message",
      {
        threadId: "thread-1",
        message: { body: "hello" },
      },
      {
        bridge: {
          bridgeAccountId: "hosted-web-user-1234",
          bridgeDeviceId: "dashboard-bridge-v1",
        },
        scope: conversationRelayScope("smoke-finite", "thread-1"),
      }
    );
    assert.deepEqual(result, { id: "msg-1" });
    assert.equal(requests[0]?.url.pathname, "/api/finite/v1/machines/smoke-finite/events");
    assert.deepEqual(requests[0]?.body, {
      lane: "chat",
      kind: "chat.send_message",
      ttlSecs: 30,
      bridge: {
        bridgeAccountId: "hosted-web-user-1234",
        bridgeDeviceId: "dashboard-bridge-v1",
      },
      scope: {
        actorDeviceId: "dashboard-bridge-v1",
        conversationId: "thread-1",
        machineId: "smoke-finite",
        runtimeId: "runtime:smoke-finite",
      },
      payload: {
        threadId: "thread-1",
        message: { body: "hello" },
      },
    });
  } finally {
    globalThis.fetch = previousFetch;
    if (previousRelayUrl === undefined) {
      delete process.env.FC_RELAY_URL;
    } else {
      process.env.FC_RELAY_URL = previousRelayUrl;
    }
    if (previousRelayToken === undefined) {
      delete process.env.FC_RELAY_ADMIN_TOKEN;
    } else {
      process.env.FC_RELAY_ADMIN_TOKEN = previousRelayToken;
    }
  }
});

test("relay command helper requires explicit matching scope", async () => {
  await assert.rejects(
    () =>
      callMachineRelay(
        "smoke-finite",
        "runtime",
        "runtime.gateway.restart",
        {},
        {} as Parameters<typeof callMachineRelay>[4]
      ),
    /relay command scope is required/
  );

  await assert.rejects(
    () =>
      callMachineRelay("smoke-finite", "runtime", "runtime.gateway.restart", {}, {
        scope: runtimeRelayScope("other-finite"),
      }),
    /relay command scope machineId mismatch/
  );
});

test("chat send helper appends through finitechat relay messages endpoint", async () => {
  const previousFetch = globalThis.fetch;
  const previousRelayUrl = process.env.FC_RELAY_URL;
  const previousRelayToken = process.env.FC_RELAY_ADMIN_TOKEN;
  const requests: Array<{ url: URL; body: unknown }> = [];

  process.env.FC_RELAY_URL = "http://127.0.0.1:4100";
  process.env.FC_RELAY_ADMIN_TOKEN = "admin-token";
  globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = new URL(String(input));
    requests.push({ url, body: init?.body ? JSON.parse(String(init.body)) : null });
    return new Response(JSON.stringify({ id: "msg-1", body: "hello" }), { status: 200 });
  }) as typeof fetch;

  try {
    const result = await sendMachineChatMessage(
      "smoke-finite",
      "thread-1",
      {
        bridgeAccountId: "hosted-web-user-1234",
        bridgeDeviceId: "dashboard-bridge-v1",
      },
      {
        body: "hello",
        client_message_id: "msg-1",
      }
    );
    assert.deepEqual(result, { id: "msg-1", body: "hello" });
    assert.equal(
      requests[0]?.url.pathname,
      "/api/finite/v1/machines/smoke-finite/chat/conversations/thread-1/messages"
    );
    assert.deepEqual(requests[0]?.body, {
      bridge: {
        bridgeAccountId: "hosted-web-user-1234",
        bridgeDeviceId: "dashboard-bridge-v1",
      },
      message: {
        body: "hello",
        client_message_id: "msg-1",
        attachments: [],
      },
    });
  } finally {
    globalThis.fetch = previousFetch;
    if (previousRelayUrl === undefined) {
      delete process.env.FC_RELAY_URL;
    } else {
      process.env.FC_RELAY_URL = previousRelayUrl;
    }
    if (previousRelayToken === undefined) {
      delete process.env.FC_RELAY_ADMIN_TOKEN;
    } else {
      process.env.FC_RELAY_ADMIN_TOKEN = previousRelayToken;
    }
  }
});

test("relay command scope rejects unsafe ids before sending", () => {
  assert.throws(
    () => conversationRelayScope("smoke-finite", "../thread"),
    /invalid relay scope conversationId/
  );
});
