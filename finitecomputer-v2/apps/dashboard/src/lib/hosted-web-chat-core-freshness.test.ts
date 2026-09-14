import assert from "node:assert/strict";
import test, { type TestContext } from "node:test";

import {
  bootstrapHostedWebChat,
  dispatchHostedWebChatAction,
  recoverHostedWebChatBinding,
} from "./hosted-web-chat";
import type { CoreMe, CoreVisibleProject } from "./core-client";

function project(runtimeId: string): CoreVisibleProject {
  return {
    project: { id: "project-1", display_name: "Moss", created_at: "2026-09-14", updated_at: "2026-09-14" },
    runtime: {
      id: runtimeId,
      project_id: "project-1",
      contact_endpoint: `https://${runtimeId}.example.test/contact`,
      runtime_status: "online",
      created_at: "2026-09-14",
      updated_at: "2026-09-14",
    },
  };
}

function snapshot(runtimeId: string): CoreMe {
  return {
    email: "owner@example.test",
    workos_user_id: "user_freshness",
    projects: [project(runtimeId)],
    agent_creation_requests: [{
      id: "creation-original",
      project_id: "project-1",
      status: "running",
      created_at: "2026-09-14",
    }],
  } as CoreMe;
}

let fixtureId = 0;

function fixture(context: TestContext) {
  const env = {
    FC_CORE_BASE_URL: `https://core-${++fixtureId}.example.test`,
    FC_DASHBOARD_ALLOW_DEV_ACCOUNT_AUTH: "1",
    FC_DASHBOARD_DEV_EMAIL: "owner@example.test",
    FC_DASHBOARD_DEV_WORKOS_USER_ID: "user_freshness",
    FC_DASHBOARD_DEV_WORKOS_ACCESS_TOKEN: "fixture-access-token",
    FC_WORKOS_AUTH_ENABLED: "false",
    FC_HOSTED_WEB_DEVICE_URL: "https://device.example.test",
    FINITECHAT_HOSTED_API_TOKEN: "fixture-device-token",
  };
  const previous = Object.fromEntries(Object.keys(env).map((name) => [name, process.env[name]]));
  const previousFetch = globalThis.fetch;
  context.after(() => {
    globalThis.fetch = previousFetch;
    for (const [name, value] of Object.entries(previous)) {
      if (value === undefined) delete process.env[name];
      else process.env[name] = value;
    }
  });
  Object.assign(process.env, env);

  const state = {
    me: snapshot("runtime-old"),
    reads: 0,
    contacts: [] as string[],
    deviceRequests: [] as Array<{ path: string; body: unknown }>,
    read: async (): Promise<Response> => Response.json(state.me),
    afterAuthorize: () => {},
  };
  globalThis.fetch = (async (input, init) => {
    const url = new URL(String(input));
    assert.equal(init?.cache, "no-store");
    if (url.pathname === "/api/core/v1/me") {
      state.reads++;
      assert.equal(new Headers(init?.headers).get("authorization"), "Bearer fixture-access-token");
      return state.read();
    }
    if (url.pathname.startsWith("/api/core/v1/me/runtime-routes/")) {
      return state.me.projects.length
        ? Response.json({ project_id: "project-1", runtime_id: state.me.projects[0]!.runtime!.id })
        : Response.json({ error: "not found" }, { status: 404 });
    }
    if (url.pathname === "/contact") {
      state.contacts.push(url.hostname);
      return Response.json({ agent_npub: `npub1${url.hostname}` });
    }
    assert.equal(url.hostname, "device.example.test");
    state.deviceRequests.push({ path: url.pathname, body: init?.body ? JSON.parse(String(init.body)) : null });
    if (url.pathname.endsWith("/open")) {
      return Response.json({ error: "missing binding" }, { status: 404 });
    }
    if (url.pathname.endsWith("/authorize-bootstrap")) state.afterAuthorize();
    return Response.json({ status: "running" });
  }) as typeof fetch;
  return state;
}

test("chat bootstrap follows Core runtime replacement instead of a cached positive route", async (context) => {
  const state = fixture(context);
  await bootstrapHostedWebChat("runtime-old");
  state.me = snapshot("runtime-new");
  await bootstrapHostedWebChat("runtime-old");
  assert.equal(state.reads, 2);
  assert.deepEqual(state.contacts, ["runtime-old.example.test", "runtime-new.example.test"]);
  assert.deepEqual(state.deviceRequests.filter((request) => request.path.endsWith("/ensure")).map((request) => request.body), [
    { project_id: "project-1", agent_npub: "npub1runtime-old.example.test", display_name: "Chat with Moss" },
    { project_id: "project-1", agent_npub: "npub1runtime-new.example.test", display_name: "Chat with Moss" },
  ]);
});

test("chat actions cannot reuse cached access after Core removes the Project", async (context) => {
  const state = fixture(context);
  await bootstrapHostedWebChat("runtime-old");
  state.me = { ...state.me, projects: [] };
  const before = state.deviceRequests.length;
  await assert.rejects(
    dispatchHostedWebChatAction("runtime-old", { StartRuntime: null }),
    { message: "Agent not found.", status: 404 }
  );
  assert.equal(state.deviceRequests.length, before);
});

test("an older in-flight Core read cannot overwrite or hold up a newer chat request", async (context) => {
  const state = fixture(context);
  const oldSnapshot = state.me;
  const entered = Promise.withResolvers<void>();
  const oldResponse = Promise.withResolvers<Response>();
  state.read = async () => {
    if (state.reads === 1) {
      entered.resolve();
      return oldResponse.promise;
    }
    return Response.json(state.me);
  };
  const oldBootstrap = bootstrapHostedWebChat("project-1");
  await entered.promise;
  state.me = snapshot("runtime-new");
  let timer: ReturnType<typeof setTimeout> | undefined;
  try {
    const current = await Promise.race([
      bootstrapHostedWebChat("project-1"),
      new Promise<"blocked">((resolve) => { timer = setTimeout(() => resolve("blocked"), 1_000); }),
    ]);
    assert.notEqual(current, "blocked", "new chat request waited for the obsolete Core read");
  } finally {
    clearTimeout(timer);
    oldResponse.resolve(Response.json(oldSnapshot));
    await oldBootstrap;
  }
  await bootstrapHostedWebChat("project-1");
  assert.equal(state.reads, 3);
  assert.deepEqual(state.contacts, ["runtime-new.example.test", "runtime-old.example.test", "runtime-new.example.test"]);
});

test("binding recovery keeps authorization and bootstrap on one fresh Core snapshot", async (context) => {
  const state = fixture(context);
  state.afterAuthorize = () => { state.me = snapshot("runtime-new"); };
  await recoverHostedWebChatBinding("runtime-old");
  assert.equal(state.reads, 1);
  assert.deepEqual(state.contacts, ["runtime-old.example.test"]);
  assert.deepEqual(state.deviceRequests.find((request) => request.path.endsWith("/authorize-bootstrap"))?.body, {
    project_id: "project-1",
    creation_request_id: "creation-original",
  });
});
