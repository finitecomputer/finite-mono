import assert from "node:assert/strict";
import test from "node:test";

import {
  AGENT_ACCOUNT_ID,
  AGENT_NPUB,
  BRIDGE_URL,
  DEVICE_URL,
  HUMAN_ACCOUNT_ID,
  directInfo,
  directWorld,
  fixtureAccess,
  fixtureBridge,
  fixtureState,
  type RecordedRequest,
} from "@/lib/direct-chat-fixture";
import {
  HostedWebChatError,
  hostedWebChatOperations,
  type HostedWebChatContext,
} from "@/lib/hosted-web-chat";
import { HostedDeviceRequestError, type HostedChatState } from "@/lib/hosted-web-device";

const context: HostedWebChatContext = {
  account: fixtureAccess.account,
  config: fixtureAccess.hosted,
  trialAccount: false,
  primaryUrl: null,
  agentName: "Moss",
  projectId: fixtureAccess.projectId,
  runtimeId: fixtureAccess.runtimeId,
};

const json = (request: RecordedRequest) => JSON.parse(new TextDecoder().decode(request.body!));
const bridgePath = (request: RecordedRequest) => {
  const url = new URL(request.url);
  return `${request.method} ${url.pathname.slice(new URL(BRIDGE_URL).pathname.length)}${url.search}`;
};

function directOperations() {
  return hostedWebChatOperations(fixtureBridge().bridge);
}

test("direct bootstrap reads agent state and derives the binding from agent info", async (t) => {
  const world = directWorld(t, {
    // The agent's AppState carries no binding; a stray null must not wipe the UI's.
    "GET /v1/app/state": () => Response.json({ ...fixtureState(), hosted_agent_binding: null }),
  }, {
    info: () => Response.json(directInfo({ rooms: ["room-1", "room-old"] })),
  });
  const state = await directOperations().bootstrap(context);
  assert.deepEqual(state.hosted_agent_binding, {
    version: 1,
    project_id: "project_1",
    human_account_id: HUMAN_ACCOUNT_ID,
    agent_account_id: AGENT_ACCOUNT_ID,
    agent_npub: AGENT_NPUB,
    canonical_room_id: "room-1",
    associated_room_ids: ["room-old"],
  });
  assert.deepEqual(world.agentRequests().map(bridgePath), ["GET /v1/app/state"]);
  assert.deepEqual(world.hostedAppRequests(), [], "no agent-binding open/ensure or hosted state");
});

test("every chat operation reaches a direct agent's bridge, never the hosted device's app API", async (t) => {
  const state = fixtureState();
  const world = directWorld(t, {
    "GET /v1/app/state": () => Response.json({ ...state, hosted_agent_binding: null }),
    "POST /v1/app/actions": () => Response.json(state),
    "POST /v1/app/new-chat": () => Response.json(state),
    "POST /v1/app/attachments": () => Response.json(state),
    "POST /v1/app/runtime-commands": () => Response.json({ request_id: "r1", status: "succeeded", body: {} }),
    "GET /v1/app/updates": () =>
      new Response("event: state\ndata: {}\n\n", { headers: { "content-type": "text/event-stream" } }),
    "GET /v1/app/attachments/room%2F1/message%201/attachment-1": () =>
      new Response("bytes", { headers: { "content-type": "image/png" } }),
  });
  const operations = directOperations();

  const view = await operations.bootstrap(context, "?room_id=room-1&topic_id=home&limit=50");
  assert.equal("hosted_agent_binding" in view, false, "transcript reads keep the bootstrap binding");
  const read = await operations.dispatch(context, { MarkRoomRead: { room_id: "room-1" } });
  assert.equal("hosted_agent_binding" in read, false);
  await operations.dispatch(context, {
    StartTopicChatIntent: { room_id: "room-1", topic_id: "home", reason: "", intent_key: "intent-1" },
  });
  const formData = new FormData();
  formData.set("room_id", "room-1");
  formData.append("files", new Blob(["file"], { type: "text/plain" }), "notes.txt");
  await operations.uploadAttachments(context, formData);
  const stream = await operations.stream(context, new AbortController().signal, "?room_id=room-1&limit=50");
  assert.equal(await stream.text(), "event: state\ndata: {}\n\n");
  const attachment = await operations.attachment(
    context,
    "room/1",
    "message 1",
    "attachment-1",
    new AbortController().signal
  );
  assert.equal(await attachment.text(), "bytes");
  assert.deepEqual(await operations.claimOwner(context), { claimed: true });
  const recovered = await operations.recoverBinding(context, []);
  assert.equal(recovered.hosted_agent_binding?.canonical_room_id, "room-1");

  const sent = world.agentRequests();
  assert.deepEqual(sent.map(bridgePath), [
    "GET /v1/app/state?room_id=room-1&topic_id=home&limit=50",
    "POST /v1/app/actions",
    "GET /v1/app/state?room_id=room-1",
    "POST /v1/app/new-chat",
    "POST /v1/app/attachments",
    "GET /v1/app/updates?room_id=room-1&limit=50",
    "GET /v1/app/attachments/room%2F1/message%201/attachment-1",
    "POST /v1/app/runtime-commands",
    "GET /v1/app/state",
  ]);
  assert.deepEqual(json(sent[1]), { MarkRoomRead: { room_id: "room-1" } });
  assert.deepEqual(json(sent[3]), {
    room_id: "room-1",
    topic_id: "home",
    reason: null,
    intent_key: "intent-1",
  });
  assert.match(sent[4].headers.get("content-type") ?? "", /^multipart\/form-data; boundary=/u);
  assert.equal(sent[5].headers.get("accept"), "text/event-stream");
  assert.deepEqual(json(sent[7]), {
    room_id: "room-1",
    target_account_id: AGENT_ACCOUNT_ID,
    command: "agent.owner.claim",
    resource_key: "agent.connections",
    schema: "finite.agent.empty.request.v1",
    body: {},
    reuse_succeeded_owner_claim: true,
    wait_millis: 45_000,
  });
  assert.equal(world.signingRequests().length, sent.length, "every bridge request is signed");
  assert.deepEqual(world.hostedAppRequests(), [], "no hosted-device chat, binding, or recovery calls");
});

test("direct text sends carry the Sites requester assertion for the signer and agent", async (t) => {
  const names = ["FC_CORE_BASE_URL", "FC_SITES_UPSTREAM_URL", "FINITE_SITES_VIEWER_SESSION_TOKEN"] as const;
  const previous = names.map((name) => process.env[name]);
  t.after(() => names.forEach((name, index) => {
    if (previous[index] === undefined) delete process.env[name];
    else process.env[name] = previous[index];
  }));
  process.env.FC_CORE_BASE_URL = "https://core.internal";
  process.env.FC_SITES_UPSTREAM_URL = "https://finite.site";
  process.env.FINITE_SITES_VIEWER_SESSION_TOKEN = "sites-service-token";
  let sitesBody: unknown = null;
  const world = directWorld(t, {
    "POST /v1/app/actions": () => Response.json(fixtureState()),
  }, {
    fallback: (request) => {
      if (request.url === "https://core.internal/api/core/v1/me") {
        return Response.json({ email: "paul@finite.vip", workos_user_id: "user_paul" });
      }
      if (request.url === "https://finite.site/internal/v1/hosted-requester-assertions") {
        sitesBody = json(request);
        return Response.json({ email: "paul@finite.vip", assertion: "assertion-1" });
      }
      return new Response(null, { status: 404 });
    },
  });
  await directOperations().dispatch(context, {
    SendChatMessage: { room_id: "room-1", topic_id: "home", chat_id: "chat-1", text: "publish it" },
  });
  assert.deepEqual(sitesBody, {
    email: "paul@finite.vip",
    requester_npub: HUMAN_ACCOUNT_ID,
    agent_npub: AGENT_NPUB,
  });
  const [sent] = world.agentRequests();
  assert.equal(sent.headers.get("x-finite-requester-email"), "paul@finite.vip");
  assert.equal(sent.headers.get("x-finite-sites-requester-assertion"), "assertion-1");
  assert.deepEqual(world.hostedAppRequests(), [], "no agent-binding open for requester context");
});

test("direct mode keeps the browser action allowlist and canonical new-chat target", async (t) => {
  const world = directWorld(t, {
    "GET /v1/app/state": () => Response.json(fixtureState()),
  });
  const operations = directOperations();
  await assert.rejects(
    operations.dispatch(context, { ScanTarget: { value: "npub1someone" } }),
    (error: unknown) => error instanceof HostedWebChatError && error.status === 400
  );
  assert.equal(world.requests.length, 0, "a refused action never resolves or reaches the agent");
  for (const target of [
    { room_id: "room-other", topic_id: "home" },
    { room_id: "room-1", topic_id: "topic-from-elsewhere" },
  ]) {
    await assert.rejects(
      operations.dispatch(context, { StartTopicChatIntent: { ...target, intent_key: "intent-1" } }),
      (error: unknown) => error instanceof HostedWebChatError && error.status === 409
    );
  }
  assert(!world.agentRequests().some((request) => request.url.endsWith("/v1/app/new-chat")));
});

test("an ambiguous direct conversation fails closed instead of guessing a room", async (t) => {
  const world = directWorld(t, {}, {
    info: () => Response.json(directInfo({ rooms: ["room-a", "room-b"], canonical_room_id: null })),
  });
  const operations = directOperations();
  for (const operation of [
    () => operations.bootstrap(context),
    () => operations.claimOwner(context),
    () => operations.dispatch(context, {
      StartTopicChatIntent: { room_id: "room-a", topic_id: "home", intent_key: "intent-1" },
    }),
  ]) {
    await assert.rejects(
      operation(),
      (error: unknown) => error instanceof HostedWebChatError && error.status === 409
    );
  }
  assert.deepEqual(world.agentRequests(), []);
});

test("bridge refusals keep their envelope for the proxy", async (t) => {
  const consoleError = console.error;
  console.error = () => {};
  t.after(() => { console.error = consoleError; });
  directWorld(t, {
    "POST /v1/app/actions": () => Response.json(
      { error: "unsupported action", error_kind: "unsupported_action", retryable: false },
      { status: 400 }
    ),
  });
  await assert.rejects(
    directOperations().dispatch(context, { RefreshDevices: null }),
    (error: unknown) =>
      error instanceof HostedDeviceRequestError &&
      error.status === 400 &&
      error.kind === "unsupported_action"
  );
});

test("a missing chat identity is minted by the hosted device once during direct bootstrap", async (t) => {
  let signatures = 0;
  const world = directWorld(t, {
    "GET /v1/app/state": () => Response.json(fixtureState()),
  }, {
    fallback: (request) => {
      if (request.url === `${DEVICE_URL}/v1/app/state`) return Response.json(fixtureState());
      return new Response(null, { status: 404 });
    },
  });
  // Wrap the world's signer: the first signature reports no identity yet.
  const worldFetch = globalThis.fetch;
  globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
    if (String(input) === `${DEVICE_URL}/v1/chat/direct-authorization` && ++signatures === 1) {
      return Response.json(
        { error: "chat identity is missing", error_kind: "chat_identity_missing", retryable: false },
        { status: 409 }
      );
    }
    return worldFetch(input, init);
  }) as typeof fetch;
  const consoleError = console.error;
  console.error = () => {};
  t.after(() => { console.error = consoleError; });

  const state = await directOperations().bootstrap(context);
  assert.equal(state.hosted_agent_binding?.human_account_id, HUMAN_ACCOUNT_ID);
  assert.deepEqual(world.hostedAppRequests().map((request) => request.url), [`${DEVICE_URL}/v1/app/state`]);
  assert.equal(signatures, 2);
});

test("trial streams stay bounded in direct mode", async (t) => {
  const world = directWorld(t, {
    "GET /v1/app/updates": () => new Response("event: state\ndata: {}\n\n"),
  });
  const operations = directOperations();
  const outer = new AbortController();
  await operations.stream(context, outer.signal);
  await operations.stream({ ...context, trialAccount: true }, outer.signal);
  const [member, trial] = world.agentRequests();
  assert.equal(member.signal, outer.signal);
  assert.notEqual(trial.signal, outer.signal, "trial streams get the 30 s access bound");
  outer.abort();
  assert.equal(trial.signal?.aborted, true);
});

test("legacy agents keep the hosted-device path unchanged", async (t) => {
  const binding = {
    version: 1,
    project_id: "project_1",
    human_account_id: "human-1",
    agent_account_id: "agent-account",
    agent_npub: "npub1agent",
    canonical_room_id: "room-1",
    associated_room_ids: [],
  };
  const bound: HostedChatState = {
    ...fixtureState(),
    rooms: [{
      room_id: "room-1",
      display_name: "Chat with Moss",
      state: "Connected",
      status: "",
      user_status_text: "",
      last_message_preview: "",
      unread_count: 0,
      can_load_older: false,
      is_agent_chat: true,
    }],
    hosted_agent_binding: binding,
  };
  const world = directWorld(t, {}, {
    info: () => Response.json({ detail: "Not Found" }, { status: 404 }),
    fallback: (request) => {
      if (request.url === `${DEVICE_URL}/v1/app/agent-bindings/open`) return Response.json(bound);
      if (request.url.startsWith(`${DEVICE_URL}/v1/app/state`)) return Response.json(fixtureState());
      if (request.url === `${DEVICE_URL}/v1/app/actions`) return Response.json(fixtureState());
      if (request.url === `${DEVICE_URL}/v1/app/new-chat`) return Response.json(fixtureState());
      if (request.url === `${DEVICE_URL}/v1/app/runtime-commands`) {
        return Response.json({ request_id: "r1", status: "succeeded" });
      }
      if (request.url.startsWith(`${DEVICE_URL}/v1/app/updates`)) return new Response("event: state\n\n");
      return new Response(null, { status: 404 });
    },
  });
  const operations = directOperations();
  assert.deepEqual((await operations.bootstrap(context)).hosted_agent_binding, binding);
  await operations.bootstrap(context, "?room_id=room-1&limit=50");
  await operations.dispatch(context, { MarkRoomRead: { room_id: "room-1" } });
  await operations.dispatch(context, {
    StartTopicChatIntent: { room_id: "room-1", topic_id: "home", intent_key: "intent-1" },
  });
  await operations.stream(context, new AbortController().signal);
  await operations.claimOwner(context);

  assert.deepEqual(
    world.hostedAppRequests().map((request) => `${request.method} ${new URL(request.url).pathname}`),
    [
      "POST /v1/app/agent-bindings/open",
      "GET /v1/app/state",
      "POST /v1/app/actions",
      "POST /v1/app/agent-bindings/open",
      "POST /v1/app/new-chat",
      "GET /v1/app/updates",
      "POST /v1/app/agent-bindings/open",
      "POST /v1/app/runtime-commands",
    ]
  );
  const newChat = world.hostedAppRequests().find((request) => request.url.endsWith("/v1/app/new-chat"))!;
  assert.equal(json(newChat).project_id, "project_1");
  for (const request of world.hostedAppRequests()) {
    assert.equal(request.headers.get("authorization"), "Bearer hosted-device-token-fixture");
    assert.equal(request.headers.get("x-finite-workos-user-id"), "user_paul");
  }
  assert.deepEqual(world.signingRequests(), [], "legacy requests are never direct-signed");
  assert.deepEqual(world.agentRequests(), [], "only the capability probe reaches the agent");
});
