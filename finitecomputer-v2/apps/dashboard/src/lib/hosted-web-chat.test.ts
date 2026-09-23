import assert from "node:assert/strict";
import { readFile, mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import {
  HostedWebChatError,
  createHostedRequesterContext,
  createNativeRequesterContext,
  hostedWebChatErrorMessage,
  hostedWebChatErrorResponse,
  isAgentBindingAuthorizationRequired,
  isCanonicalNewChatTarget,
  parseHostedChatAction,
  selectOriginalAgentCreationRequest,
} from "@/lib/hosted-web-chat";
import type { CoreAgentCreationRequestSummary } from "@/lib/core-client";
import { CHAT_UNAVAILABLE_MESSAGE } from "@/lib/chat-product-copy";
import {
  HostedDeviceRequestError,
  type HostedChatState,
} from "@/lib/hosted-web-device";

test("verified hosted requester assertion binds mailbox, human, and agent", async (context) => {
  const originalFetch = global.fetch;
  const originalCore = process.env.FC_CORE_BASE_URL;
  const originalUpstream = process.env.FC_SITES_UPSTREAM_URL;
  const originalToken = process.env.FINITE_SITES_VIEWER_SESSION_TOKEN;
  context.after(() => {
    global.fetch = originalFetch;
    if (originalCore === undefined) delete process.env.FC_CORE_BASE_URL;
    else process.env.FC_CORE_BASE_URL = originalCore;
    if (originalUpstream === undefined) delete process.env.FC_SITES_UPSTREAM_URL;
    else process.env.FC_SITES_UPSTREAM_URL = originalUpstream;
    if (originalToken === undefined) delete process.env.FINITE_SITES_VIEWER_SESSION_TOKEN;
    else process.env.FINITE_SITES_VIEWER_SESSION_TOKEN = originalToken;
  });
  process.env.FC_CORE_BASE_URL = "https://core.internal";
  process.env.FC_SITES_UPSTREAM_URL = "https://finite.site";
  process.env.FINITE_SITES_VIEWER_SESSION_TOKEN = "sites-token";
  const requests: Array<{ url: string; body: unknown }> = [];
  global.fetch = (async (input, init) => {
    const url = String(input);
    requests.push({
      url,
      body: init?.body ? JSON.parse(String(init.body)) : null,
    });
    if (url.endsWith("/api/core/v1/me")) {
      return Response.json({ email: "after@example.test", workos_user_id: "user_fixture" });
    }
    if (url.endsWith("/v1/app/state")) {
      const state = targetState();
      delete state.hosted_agent_binding;
      return Response.json(state);
    }
    if (url.endsWith("/v1/app/agent-bindings/open")) {
      assert.equal(init?.method, "POST");
      assert.deepEqual(JSON.parse(String(init?.body)), { project_id: "project-1" });
      return Response.json(targetState());
    }
    return Response.json({
      email: "after@example.test",
      requester_npub: "npub1human",
      agent_npub: "npub1agent",
      assertion: "assertion-1",
      expires_at: 123,
    });
  }) as typeof fetch;

  const requester = await createHostedRequesterContext({
    projectId: "project-1",
    config: { baseUrl: "https://device.internal", apiToken: "device-token" },
    account: {
      email: "before@example.test",
      accessToken: "fixture-access-token",
      workosUserId: "user_fixture",
      emailVerified: true,
      source: "workos",
    },
  });
  assert.deepEqual(requester, {
    email: "after@example.test",
    sitesAssertion: "assertion-1",
    userId: "human-1",
    expiresAt: 123,
  });
  assert.equal(requests[1].url, "https://device.internal/v1/app/agent-bindings/open");
  assert.deepEqual(requests[2], {
    url: "https://finite.site/internal/v1/hosted-requester-assertions",
    body: {
      email: "after@example.test",
      requester_npub: "human-1",
      agent_npub: "npub1agent",
    },
  });
});

test("requester assertions never fall back to stale session email when Core fails or returns another subject", async (context) => {
  const originalFetch = global.fetch;
  const names = ["FC_CORE_BASE_URL", "FC_SITES_UPSTREAM_URL", "FINITE_SITES_VIEWER_SESSION_TOKEN"] as const;
  const previous = names.map((name) => process.env[name]);
  context.after(() => {
    global.fetch = originalFetch;
    names.forEach((name, i) => {
      if (previous[i] === undefined) delete process.env[name];
      else process.env[name] = previous[i];
    });
  });
  process.env.FC_CORE_BASE_URL = "https://core.internal";
  process.env.FC_SITES_UPSTREAM_URL = "https://finite.site";
  process.env.FINITE_SITES_VIEWER_SESSION_TOKEN = "fixture-service-token";
  for (const response of [
    Response.json({ error: "unavailable" }, { status: 503 }),
    Response.json({ email: "other@example.test", workos_user_id: "user_other" }),
  ]) {
    const requests: string[] = [];
    global.fetch = (async (input) => { requests.push(String(input)); return response; }) as typeof fetch;
    const result = await createHostedRequesterContext({
      projectId: "project-1",
      config: { baseUrl: "https://device.internal", apiToken: "fixture-device-token" },
      account: { email: "before@example.test", workosUserId: "user_fixture", emailVerified: true, accessToken: "fixture-access-token", source: "workos" },
    });
    assert.equal(result, undefined);
    assert.deepEqual(requests, ["https://core.internal/api/core/v1/me"]);
  }
});

test("missing or failing Sites requester exchange keeps Chat context optional", async (context) => {
  const previous = { ...process.env };
  const originalFetch = global.fetch;
  context.after(() => { process.env = previous; global.fetch = originalFetch; });
  process.env.FC_CORE_BASE_URL = "https://core.internal";
  process.env.FC_SITES_UPSTREAM_URL = "https://legacy.internal";
  process.env.FINITE_SITES_VIEWER_SESSION_TOKEN = "fixture-service-token";
  const input = {
    projectId: "project-1",
    config: { baseUrl: "https://device.internal", apiToken: "device-token" },
    account: { email: "person@example.test", workosUserId: "user_fixture", emailVerified: true,
      accessToken: "fixture-access-token", source: "workos" as const },
  };
  const requests: string[] = [];
  global.fetch = (async (url) => {
    requests.push(String(url));
    throw new Error("missing configuration must not fetch");
  }) as typeof fetch;
  for (const origin of [undefined, "", "https://finite.site/internal", "file:///tmp/sites"]) {
    if (origin === undefined) delete process.env.FC_SITES_UPSTREAM_URL;
    else process.env.FC_SITES_UPSTREAM_URL = origin;
    assert.equal(await createHostedRequesterContext(input), undefined);
  }
  assert.equal(requests.length, 0);

  process.env.FC_SITES_UPSTREAM_URL = "https://finite.site";
  for (const failure of ["unavailable", "unauthorized", "disconnect", "bad-payload"] as const) {
    requests.length = 0;
    global.fetch = (async (url, init) => {
      requests.push(String(url));
      if (String(url).endsWith("/api/core/v1/me")) {
        return Response.json({ email: input.account.email, workos_user_id: input.account.workosUserId });
      }
      if (String(url).endsWith("/v1/app/agent-bindings/open")) return Response.json(targetState());
      assert.equal(String(url), "https://finite.site/internal/v1/hosted-requester-assertions");
      assert.equal(init?.redirect, "error");
      if (failure === "disconnect") throw new Error("network down");
      if (failure === "bad-payload") return Response.json({ email: "other@example.test", assertion: "wrong-subject" });
      return new Response(null, { status: failure === "unavailable" ? 503 : 401 });
    }) as typeof fetch;
    assert.equal(await createHostedRequesterContext(input), undefined);
    assert.equal(requests.length, 3);
    assert(!requests.some((url) => url.includes("legacy.internal")));
  }
  for (const invalid of ["missing", "other-project", "other-human", "unavailable"] as const) {
    requests.length = 0;
    global.fetch = (async (url) => {
      requests.push(String(url));
      if (String(url).endsWith("/api/core/v1/me")) {
        return Response.json({ email: input.account.email, workos_user_id: input.account.workosUserId });
      }
      assert.equal(String(url), "https://device.internal/v1/app/agent-bindings/open");
      if (invalid === "unavailable") return new Response(null, { status: 503 });
      const state = targetState();
      if (invalid === "missing") delete state.hosted_agent_binding;
      if (invalid === "other-project") state.hosted_agent_binding!.project_id = "project-other";
      if (invalid === "other-human") state.hosted_agent_binding!.human_account_id = "other-human";
      return Response.json(state);
    }) as typeof fetch;
    assert.equal(await createHostedRequesterContext(input), undefined);
    assert.equal(requests.length, 2);
  }
});

test("native requester survives Sites outage but requires the exact human and Project", async (t) => {
  const saved = { ...process.env };
  t.after(() => { process.env = saved; });
  const root = await mkdtemp(join(tmpdir(), "native-requester-"));
  t.after(() => rm(root, { recursive: true, force: true }));
  process.env = { ...process.env, NODE_ENV: "development", PATH: "", FC_FINITED_BIN: "",
    FC_REPO_ROOT: root, FC_WORKSPACE_ROOT: root, FC_CONTROL_PLANE_ROOT: root,
    FC_WORKOS_AUTH_ENABLED: "0", FC_DASHBOARD_ALLOW_DEV_ACCOUNT_AUTH: "1",
    FC_DASHBOARD_DEV_EMAIL: "owner@example.test", FC_DASHBOARD_DEV_WORKOS_USER_ID: "native-owner",
    FC_DASHBOARD_DEV_WORKOS_ACCESS_TOKEN: "local-fixture", FC_CORE_BASE_URL: "https://core.test",
    FC_HOSTED_WEB_DEVICE_URL: "https://device.test", FINITECHAT_HOSTED_API_TOKEN: "local-device-fixture",
    FC_SITES_UPSTREAM_URL: "https://sites.test", FINITE_SITES_VIEWER_SESSION_TOKEN: "local-sites-fixture",
  };
  const user = "a1".repeat(32);
  let bindingHuman = user, bindingProject = "project-1", sitesCalls = 0;
  t.mock.method(globalThis, "fetch", async (input: string | URL) => {
    const url = String(input);
    if (url === "https://core.test/api/core/v1/me") return Response.json({
      workos_user_id: "native-owner", email: "owner@example.test", projects: [{
        project: { id: "project-1", display_name: "Native" }, runtime: { id: "runtime-1" },
      }],
    });
    if (url === "https://device.test/v1/app/agent-bindings/open") {
      const state = targetState(); state.identity.account_id = user;
      state.hosted_agent_binding!.human_account_id = bindingHuman;
      state.hosted_agent_binding!.project_id = bindingProject;
      return Response.json(state);
    }
    if (url === "https://sites.test/internal/v1/hosted-requester-assertions") {
      sitesCalls += 1; return Response.json({ error: "unavailable" }, { status: 503 });
    }
    throw new Error(`Unexpected request: ${url}`);
  });
  const before = Math.floor(Date.now() / 1000);
  const identity = await createNativeRequesterContext("runtime-1");
  assert.equal(identity?.userId, user);
  assert.ok(identity && identity.expiresAt >= before + 600 && identity.expiresAt <= Math.floor(Date.now() / 1000) + 600);
  assert.deepEqual(Object.keys(identity!).sort(), ["expiresAt", "userId"]);
  assert.equal(sitesCalls, 1);
  delete process.env.FC_SITES_UPSTREAM_URL;
  assert.equal((await createNativeRequesterContext("runtime-1"))?.userId, user);
  assert.equal(sitesCalls, 1);
  bindingHuman = "c3".repeat(32);
  assert.equal(await createNativeRequesterContext("runtime-1"), undefined);
  bindingHuman = user; bindingProject = "other-project";
  assert.equal(await createNativeRequesterContext("runtime-1"), undefined);
});

function targetState(): HostedChatState {
  return {
    rev: 1,
    identity: { account_id: "human-1", device_id: "hosted-web" },
    rooms: [],
    topics: [{
      room_id: "canonical-room",
      topic_id: "home",
      title: "Home",
      last_message_preview: "",
      unread_count: 0,
      message_count: 0,
      created_seq: 0,
      updated_seq: 0,
      archived: false,
      chats: [],
    }],
    status: "ready",
    messages: [],
    profiles: [],
    devices: [],
    typing_members: [],
    hosted_agent_binding: {
      version: 1,
      project_id: "project-1",
      human_account_id: "human-1",
      agent_account_id: "agent-1",
      agent_npub: "npub1agent",
      canonical_room_id: "canonical-room",
      associated_room_ids: ["legacy-room"],
    },
    flow: {
      notice_busy: false,
      scan_in_flight: false,
      scan_result: "",
    },
  };
}

test("new chat validation rejects legacy and cross-room topic targets", () => {
  const state = targetState();
  assert.equal(isCanonicalNewChatTarget(state, {
    room_id: "canonical-room",
    topic_id: "home",
    intent_key: "intent-1",
  }), true);
  assert.equal(isCanonicalNewChatTarget(state, {
    room_id: "legacy-room",
    topic_id: "home",
    intent_key: "intent-2",
  }), false);
  assert.equal(isCanonicalNewChatTarget(state, {
    room_id: "canonical-room",
    topic_id: "legacy-topic",
    intent_key: "intent-3",
  }), false);
});

test("unexpected chat infrastructure errors are replaced with plain product copy", () => {
  assert.equal(
    hostedWebChatErrorMessage(new Error("Hosted Web Device update stream failed")),
    CHAT_UNAVAILABLE_MESSAGE
  );
  assert.equal(
    hostedWebChatErrorMessage(new HostedWebChatError("Sign in again to use chat.", 401)),
    "Sign in again to use chat."
  );
});

test("hosted-device authorization recovery recognizes its service-unavailable contract", () => {
  assert.equal(
    isAgentBindingAuthorizationRequired(
      new HostedDeviceRequestError(
        "canonical Agent conversation requires recovery: first-time binding bootstrap was not authorized by Project creation",
        503
      )
    ),
    true
  );
  assert.equal(
    isAgentBindingAuthorizationRequired(
      new HostedDeviceRequestError(
        "the observed Agent Principal changed",
        503
      )
    ),
    false
  );
  assert.equal(
    isAgentBindingAuthorizationRequired(
      new HostedDeviceRequestError(
        "first-time binding bootstrap was not authorized by Project creation",
        409
      )
    ),
    true
  );
  assert.equal(
    isAgentBindingAuthorizationRequired(
      new HostedDeviceRequestError(
        "first-time binding bootstrap was not authorized by Project creation",
        503
      )
    ),
    false
  );
});

test("parseHostedChatAction accepts the bounded message operations used by web chat", () => {
  assert.deepEqual(
    parseHostedChatAction({
      StartTopicChatIntent: {
        room_id: "room-1",
        topic_id: "topic-1",
        reason: null,
        intent_key: "intent-1",
      },
    }),
    {
      StartTopicChatIntent: {
        room_id: "room-1",
        topic_id: "topic-1",
        reason: null,
        intent_key: "intent-1",
      },
    }
  );

  assert.deepEqual(
    parseHostedChatAction({
      SendChatMessage: {
        room_id: "room-1",
        topic_id: "topic-1",
        chat_id: "chat-1",
        text: "hello",
      },
    }),
    {
      SendChatMessage: {
        room_id: "room-1",
        topic_id: "topic-1",
        chat_id: "chat-1",
        text: "hello",
        metadata_json: null,
      },
    }
  );

  assert.deepEqual(
    parseHostedChatAction({
      SendChatMessage: {
        room_id: "room-1",
        topic_id: "topic-1",
        chat_id: "chat-1",
        text: "approved",
        metadata_json: JSON.stringify({
          approve: { service: "brain", choice: "approved", requests: [] },
        }),
      },
    }),
    {
      SendChatMessage: {
        room_id: "room-1",
        topic_id: "topic-1",
        chat_id: "chat-1",
        text: "approved",
        metadata_json: JSON.stringify({
          approve: { service: "brain", choice: "approved", requests: [] },
        }),
      },
    }
  );

  assert.throws(() =>
    parseHostedChatAction({
      SendChatMessage: {
        room_id: "room-1",
        topic_id: "topic-1",
        chat_id: "chat-1",
        text: "hello",
        metadata_json: "not json",
      },
    })
  );

  assert.deepEqual(
    parseHostedChatAction({
      RenameChat: {
        room_id: "room-1",
        topic_id: "topic-1",
        chat_id: "chat-1",
        title: "Launch checklist",
      },
    }),
    {
      RenameChat: {
        room_id: "room-1",
        topic_id: "topic-1",
        chat_id: "chat-1",
        title: "Launch checklist",
      },
    }
  );

  assert.deepEqual(
    parseHostedChatAction({
      SetChatArchived: {
        room_id: "room-1",
        topic_id: "topic-1",
        chat_id: "chat-1",
        archived: true,
      },
    }),
    {
      SetChatArchived: {
        room_id: "room-1",
        topic_id: "topic-1",
        chat_id: "chat-1",
        archived: true,
      },
    }
  );

  assert.deepEqual(
    parseHostedChatAction({
      SetTyping: { room_id: "room-1", is_typing: true },
    }),
    { SetTyping: { room_id: "room-1", is_typing: true } }
  );

  assert.deepEqual(parseHostedChatAction({ RefreshDevices: null }), {
    RefreshDevices: null,
  });
  assert.deepEqual(
    parseHostedChatAction({
      RevokeDevice: { account_id: "account-1", device_id: "electron-alpha" },
    }),
    { RevokeDevice: { account_id: "account-1", device_id: "electron-alpha" } }
  );
});

test("parseHostedChatAction keeps pairing and unsupported operations off the browser action surface", () => {
  assert.throws(
    () =>
      parseHostedChatAction({
        StartTopicChat: { room_id: "legacy-room", topic_id: "home", reason: null },
      }),
    /Unsupported chat action/
  );
  assert.throws(
    () => parseHostedChatAction({ ScanTarget: { value: "finite://join?secret" } }),
    (error: unknown) =>
      error instanceof HostedWebChatError
      && error.status === 400
      && /Unsupported chat action/.test(error.message)
  );
  assert.throws(
    () =>
      parseHostedChatAction({
        StartProfileChat: { profile: {}, display_name: "Injected room" },
      }),
    /Unsupported chat action/
  );
  assert.throws(
    () =>
      parseHostedChatAction({
        StartGroupChat: { profiles: [], display_name: "Injected recovery room" },
      }),
    /Unsupported chat action/
  );
  assert.throws(
    () => parseHostedChatAction({ DeleteEverything: null }),
    /Unsupported chat action/
  );
});

test("parseHostedChatAction requires explicit archive state", () => {
  assert.throws(
    () =>
      parseHostedChatAction({
        SetChatArchived: {
          room_id: "room-1",
          topic_id: "topic-1",
          chat_id: "chat-1",
          archived: "yes",
        },
      }),
    /archived must be a boolean/
  );
});

test("parseHostedChatAction rejects ambiguous and oversized input", () => {
  assert.throws(
    () => parseHostedChatAction({ StartRuntime: null, OpenRoom: { room_id: "room-1" } }),
    /exactly one operation/
  );
  assert.throws(
    () =>
      parseHostedChatAction({
        SendMessage: { room_id: "room-1", text: "x".repeat(70 * 1024) },
      }),
    /Invalid text/
  );
  assert.throws(
    () =>
      parseHostedChatAction({
        LoadOlderMessages: {
          room_id: "room-1",
          before_message_id: "message-1",
          limit: 101,
        },
      }),
    /Invalid limit/
  );
  assert.throws(
    () => parseHostedChatAction({ SetTyping: { room_id: "room-1", is_typing: "yes" } }),
    /Invalid is_typing/
  );
});

test("a currency-gate refusal passes through the proxy with its classification", () => {
  const behindMessage =
    "device state for room room-1 is behind the server (own-send mark 2, server holds this device's entry at seq 3); the store was rewound and sends are refused until a commit advances the room epoch";
  assert.deepEqual(
    hostedWebChatErrorResponse(
      new HostedDeviceRequestError(behindMessage, 409, "currency_behind", false)
    ),
    {
      status: 409,
      body: { error: behindMessage, error_kind: "currency_behind", retryable: false },
    }
  );
  const unverifiedMessage =
    "room room-1 currency is unverified: no sync tick has completed for it since the store was opened";
  assert.deepEqual(
    hostedWebChatErrorResponse(
      new HostedDeviceRequestError(unverifiedMessage, 503, "currency_unverified", true)
    ),
    {
      status: 503,
      body: { error: unverifiedMessage, error_kind: "currency_unverified", retryable: true },
    }
  );
});

test("failures without a hosted-device envelope still read as the generic outage", () => {
  assert.deepEqual(
    hostedWebChatErrorResponse(new HostedDeviceRequestError("lock poisoned", 500)),
    { status: 502, body: { error: CHAT_UNAVAILABLE_MESSAGE } }
  );
  assert.deepEqual(hostedWebChatErrorResponse(new Error("update stream failed")), {
    status: 502,
    body: { error: CHAT_UNAVAILABLE_MESSAGE },
  });
  assert.deepEqual(
    hostedWebChatErrorResponse(new HostedWebChatError("Sign in again to use chat.", 401)),
    { status: 401, body: { error: "Sign in again to use chat." } }
  );
});

test("the actions proxy route answers failures through the one error response", async () => {
  const routeSource = await readFile(
    new URL(
      "../app/api/chat/machines/[machineId]/hosted-device/actions/route.ts",
      import.meta.url
    ),
    "utf8"
  );
  assert.match(routeSource, /const \{ status, body \} = hostedWebChatErrorResponse\(error\)/u);
  assert.match(routeSource, /Response\.json\(body, \{ status \}\)/u);
});

function creationSummary(overrides: Partial<CoreAgentCreationRequestSummary>) {
  return {
    id: "agent_request_base",
    project_id: "project_base",
    display_name: "Agent",
    status: "running",
    agent_runtime_id: null,
    failure_message: null,
    created_at: "2026-07-12T00:00:00Z",
    updated_at: "2026-07-12T00:00:00Z",
    ...overrides,
  } satisfies CoreAgentCreationRequestSummary;
}

test("binding recovery authorizes the one original request, ignoring relocation rows", () => {
  const original = creationSummary({
    id: "agent_request_original",
    project_id: "project_boss",
    created_at: "2026-07-12T09:00:00Z",
  });
  const firstRelocation = creationSummary({
    id: "agent_request_relocation_aug",
    project_id: "project_boss",
    is_relocation: true,
    agent_runtime_id: "runtime_boss",
    created_at: "2026-08-29T21:00:00Z",
  });
  const secondRelocation = creationSummary({
    id: "agent_request_relocation_sep",
    project_id: "project_boss",
    is_relocation: true,
    agent_runtime_id: "runtime_boss",
    created_at: "2026-09-02T15:00:00Z",
  });
  const otherProject = creationSummary({
    id: "agent_request_other",
    project_id: "project_other",
    created_at: "2026-06-01T00:00:00Z",
  });
  assert.equal(
    selectOriginalAgentCreationRequest(
      [otherProject, secondRelocation, original, firstRelocation],
      "project_boss"
    )?.id,
    "agent_request_original",
    "the project's single live ORIGINAL request must win; relocation rows are history, not authority"
  );
  assert.equal(
    selectOriginalAgentCreationRequest([original], "project_boss")?.id,
    "agent_request_original",
    "a single live original request still selects itself"
  );
});

test("binding recovery selection fails closed on relocation rows without an original", () => {
  const firstRelocation = creationSummary({
    id: "agent_request_relocation_aug",
    project_id: "project_no_original",
    is_relocation: true,
    agent_runtime_id: "runtime_no_original",
    created_at: "2026-08-29T21:00:00Z",
  });
  const secondRelocation = creationSummary({
    id: "agent_request_relocation_sep",
    project_id: "project_no_original",
    is_relocation: true,
    agent_runtime_id: "runtime_no_original",
    created_at: "2026-09-02T15:00:00Z",
  });
  assert.equal(
    selectOriginalAgentCreationRequest(
      [firstRelocation, secondRelocation],
      "project_no_original"
    ),
    null,
    "no original request means no authorization: timestamp order must never pick a relocation"
  );
  assert.equal(
    selectOriginalAgentCreationRequest([firstRelocation], "project_no_original"),
    null,
    "even a single relocation row is not the creation that authored the project"
  );
});

test("binding recovery selection fails closed on ambiguous originals", () => {
  const first = creationSummary({
    id: "agent_request_first",
    project_id: "project_ambiguous",
    created_at: "2026-07-12T09:00:00Z",
  });
  const second = creationSummary({
    id: "agent_request_second",
    project_id: "project_ambiguous",
    created_at: "2026-09-02T15:00:00Z",
  });
  assert.equal(
    selectOriginalAgentCreationRequest([first, second], "project_ambiguous"),
    null,
    "more than one live original is ambiguous evidence and must stay a recovery error"
  );
  assert.equal(
    selectOriginalAgentCreationRequest([second, first], "project_ambiguous"),
    null,
    "the ambiguity must not depend on input order"
  );
});

test("binding recovery selection fails closed without a live request", () => {
  const cancelled = creationSummary({
    id: "agent_request_cancelled",
    project_id: "project_dead",
    status: "cancelled",
  });
  const failed = creationSummary({
    id: "agent_request_failed",
    project_id: "project_dead",
    status: "failed",
  });
  assert.equal(
    selectOriginalAgentCreationRequest([cancelled, failed], "project_dead"),
    null,
    "only requested/launching/running requests may authorize recovery"
  );
  assert.equal(selectOriginalAgentCreationRequest([], "project_dead"), null);
  assert.equal(
    selectOriginalAgentCreationRequest(
      [creationSummary({ project_id: "project_other" })],
      "project_dead"
    ),
    null,
    "another project's live request must not authorize this project"
  );
  const cancelledOriginal = creationSummary({
    id: "agent_request_cancelled_original",
    project_id: "project_mixed",
    status: "cancelled",
  });
  const liveRelocation = creationSummary({
    id: "agent_request_live_relocation",
    project_id: "project_mixed",
    is_relocation: true,
    status: "running",
  });
  assert.equal(
    selectOriginalAgentCreationRequest(
      [cancelledOriginal, liveRelocation],
      "project_mixed"
    ),
    null,
    "a dead original cannot be replaced by a live relocation row"
  );
});
