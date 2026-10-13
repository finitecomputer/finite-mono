import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import test from "node:test";

import { CoreFetchError } from "@/lib/core-client";
import {
  AGENT_ACCOUNT_ID,
  AGENT_NPUB,
  BRIDGE_URL,
  DEVICE_URL,
  HOSTED_API_TOKEN,
  NATIVE_BASE_URL,
  WORKOS_ACCESS_TOKEN,
  directInfo,
  directWorld,
  fixtureAccess,
  fixtureBridge,
  fixtureState,
  installFetch,
} from "@/lib/direct-chat-fixture";
import {
  DIRECT_CHAT_AUTHORIZATION_HEADER,
  classifyDirectChatInfo,
  directChatAgentId,
  directChatCanonicalRoomId,
  directChatEnabledFor,
  encodeMultipartBody,
  npubFromAccountId,
  type DirectChatTarget,
} from "@/lib/direct-chat-bridge";
import { HostedWebChatError } from "@/lib/hosted-web-chat-error";
import { HostedDeviceRequestError } from "@/lib/hosted-web-device";

const silentConsole = (context: { after: (fn: () => void) => void }) => {
  const error = console.error;
  const warn = console.warn;
  console.error = () => {};
  console.warn = () => {};
  context.after(() => {
    console.error = error;
    console.warn = warn;
  });
};

async function directTarget(bridge: ReturnType<typeof fixtureBridge>["bridge"]) {
  const target = await bridge.resolve(fixtureAccess);
  assert.equal(target.kind, "direct");
  return target as DirectChatTarget;
}

test("an agent reporting direct transport selects its bridge and the decision is reused for 30 s", async (context) => {
  const world = directWorld(context, {});
  const { bridge, mints, advance } = fixtureBridge();

  const target = await directTarget(bridge);
  assert.deepEqual(target.info, {
    version: 1,
    agentId: "agent_project_1",
    agentAccountId: AGENT_ACCOUNT_ID,
    rooms: ["room-1"],
    canonicalRoomId: "room-1",
    migrationState: "migrated",
  });
  assert.equal(world.requests.length, 1);
  const [info] = world.requests;
  assert.equal(info.url, `${NATIVE_BASE_URL}api/plugins/finitechat/v1/app/info`);
  assert.equal(info.headers.get("authorization"), "Bearer native-token-1");
  assert.equal(info.headers.get(DIRECT_CHAT_AUTHORIZATION_HEADER), null);

  advance(29_000);
  assert.equal((await bridge.resolve(fixtureAccess)).kind, "direct");
  assert.equal(world.requests.length, 1, "a fresh decision is not re-probed");

  advance(2_000);
  assert.equal((await bridge.resolve(fixtureAccess)).kind, "direct");
  assert.equal(world.requests.length, 2, "a stale decision is re-probed");
  assert.equal(mints.length, 1, "the probe reused the cached native grant");
});

test("concurrent first requests share one probe", async (context) => {
  const world = directWorld(context, {});
  const { bridge, mints } = fixtureBridge();
  const targets = await Promise.all([1, 2, 3].map(() => bridge.resolve(fixtureAccess)));
  assert(targets.every((target) => target.kind === "direct"));
  assert.equal(world.requests.length, 1);
  assert.equal(mints.length, 1);
});

test("agents that say they are not direct, or that this owner cannot reach, stay legacy", async (context) => {
  const cases: Array<[string, { info?: () => Response; mint?: () => never }]> = [
    ["plugin route missing", { info: () => Response.json({ detail: "Not Found" }, { status: 404 }) }],
    ["older image: platform plugin without a dashboard API", {
      // hermes 0.21 `hermes serve` catch-all for an enabled plugin with no routes.
      info: () => Response.json(
        { error: "Headless backend (hermes serve): web UI disabled — use `hermes dashboard` for the browser UI." },
        { status: 404 }
      ),
    }],
    ["sidecar serving MLS chat", {
      info: () => Response.json(
        { error: "direct chat is not enabled", error_kind: "not_found", retryable: false },
        { status: 404 }
      ),
    }],
    ["MLS transport", { info: () => Response.json({ transport: "mls", version: 1 }) }],
    ["pending fold", { info: () => Response.json(directInfo({ migration: { state: "pending", source: null } })) }],
    ["not this owner's runtime", { mint: () => { throw new CoreFetchError("agent runtime was not found", 404); } }],
  ];
  for (const [label, scenario] of cases) {
    directWorld(context, {}, { info: scenario.info });
    const { bridge } = fixtureBridge({ mint: scenario.mint });
    assert.deepEqual(await bridge.resolve(fixtureAccess), { kind: "legacy" }, label);
  }
});

test("a legacy decision is cached and needs no Core grant while fresh", async (context) => {
  const world = directWorld(context, {}, {
    info: () => Response.json({ detail: "Not Found" }, { status: 404 }),
  });
  const { bridge, mints, advance } = fixtureBridge();
  assert.equal((await bridge.resolve(fixtureAccess)).kind, "legacy");
  advance(10_000);
  assert.equal((await bridge.resolve(fixtureAccess)).kind, "legacy");
  assert.equal(world.requests.length, 1);
  assert.equal(mints.length, 1);
});

test("failures that are not evidence never route an unknown agent to legacy", async (context) => {
  silentConsole(context);
  const cases: Array<[string, { info?: () => Response; mint?: () => never }]> = [
    ["bridge unavailable", {
      info: () => Response.json(
        { error: "Finite Chat bridge is unavailable", error_kind: "bridge_unavailable", retryable: true },
        { status: 503 }
      ),
    }],
    ["agent network failure", { info: () => { throw new TypeError("fetch failed"); } }],
    ["native rejects a fresh grant", { info: () => new Response(null, { status: 401 }) }],
    ["Core cannot reach native Hermes", { mint: () => { throw new CoreFetchError("native Hermes is unavailable", 503); } }],
    ["Core transport failure", { mint: () => { throw new TypeError("fetch failed"); } }],
    ["hosted access not enabled", { mint: () => { throw new CoreFetchError("hosted access is not enabled", 409); } }],
    ["ingress has no route", { info: () => new Response(null, { status: 404 }) }],
    ["a 404 envelope that is not not_found", {
      info: () => Response.json(
        { error: "Finite Chat bridge is unavailable", error_kind: "bridge_unavailable", retryable: true },
        { status: 404 }
      ),
    }],
    ["non-JSON page", { info: () => new Response("<!doctype html>", { headers: { "content-type": "text/html" } }) }],
    ["JSON without transport", { info: () => Response.json({ ok: true }) }],
  ];
  for (const [label, scenario] of cases) {
    directWorld(context, {}, { info: scenario.info });
    const { bridge } = fixtureBridge({ mint: scenario.mint });
    await assert.rejects(
      bridge.resolve(fixtureAccess),
      (error: unknown) => error instanceof HostedWebChatError && error.status === 503,
      label
    );
  }
});

test("an agent seen direct is never sent to legacy by later indirect evidence or outages", async (context) => {
  silentConsole(context);
  let mode: "direct" | "outage" | "html" | "mls" = "direct";
  let hostedAccess = true;
  directWorld(context, {}, {
    info: () => {
      if (mode === "outage") return new Response("bad gateway", { status: 502 });
      if (mode === "html") return new Response("<html></html>");
      if (mode === "mls") return Response.json({ transport: "mls" });
      return Response.json(directInfo());
    },
  });
  const { bridge, advance } = fixtureBridge({
    mint: () => {
      if (!hostedAccess) throw new CoreFetchError("hosted access is not enabled", 409);
      return undefined;
    },
    grantLifetimeSeconds: 5,
  });
  assert.equal((await bridge.resolve(fixtureAccess)).kind, "direct");

  for (const next of ["outage", "html"] as const) {
    mode = next;
    advance(31_000);
    await assert.rejects(
      bridge.resolve(fixtureAccess),
      (error: unknown) => error instanceof HostedWebChatError && error.status === 503,
      next
    );
  }

  mode = "direct";
  hostedAccess = false;
  advance(31_000);
  await assert.rejects(
    bridge.resolve(fixtureAccess),
    (error: unknown) => error instanceof HostedWebChatError && error.status === 503,
    "Core hosted-access refusal"
  );

  // The agent's own report is authoritative: an explicit MLS answer is legacy.
  hostedAccess = true;
  mode = "mls";
  advance(31_000);
  assert.equal((await bridge.resolve(fixtureAccess)).kind, "legacy");
});

test("an outage keeps a legacy agent on legacy instead of breaking its chat", async (context) => {
  let outage = false;
  const world = directWorld(context, {}, {
    info: () => outage ? new Response(null, { status: 503 }) : Response.json({ transport: "mls" }),
  });
  const { bridge, advance } = fixtureBridge();
  assert.equal((await bridge.resolve(fixtureAccess)).kind, "legacy");
  outage = true;
  advance(31_000);
  assert.equal((await bridge.resolve(fixtureAccess)).kind, "legacy");
  advance(10_000);
  assert.equal((await bridge.resolve(fixtureAccess)).kind, "legacy");
  assert.equal(world.requests.length, 2, "the outage answer is reused for another window");
});

test("direct info that does not describe this agent fails closed", async (context) => {
  silentConsole(context);
  for (const [label, info] of [
    ["another agent", directInfo({ agent_id: "agent_project_other" })],
    ["bad agent account", directInfo({ agent_account_id: "npub1agent" })],
    ["canonical room missing from rooms", directInfo({ canonical_room_id: "room-9" })],
    ["unknown version", directInfo({ version: 2 })],
    ["rooms not a list", directInfo({ rooms: "room-1" })],
    ["duplicate rooms", directInfo({ rooms: ["room-1", "room-1"] })],
    ["unknown migration", directInfo({ migration: { state: "later" } })],
  ] as const) {
    directWorld(context, {}, { info: () => Response.json(info) });
    const { bridge } = fixtureBridge();
    await assert.rejects(
      bridge.resolve(fixtureAccess),
      (error: unknown) => error instanceof HostedWebChatError && error.status === 503,
      label
    );
  }
});

test("Core rejecting the account session asks the user to sign in again", async (context) => {
  directWorld(context, {});
  const { bridge } = fixtureBridge({
    mint: () => { throw new CoreFetchError("unauthorized", 401); },
  });
  await assert.rejects(
    bridge.resolve(fixtureAccess),
    (error: unknown) => error instanceof HostedWebChatError && error.status === 401
  );
});

test("native grants are cached per user and runtime until expiresAt minus 10 s", async (context) => {
  directWorld(context, { "GET /v1/app/state": () => Response.json(fixtureState()) });
  const { bridge, mints, advance } = fixtureBridge({ grantLifetimeSeconds: 60 });
  const target = await directTarget(bridge);
  const read = () => bridge.json(fixtureAccess, target, { method: "GET", path: "/v1/app/state" });

  await read();
  advance(49_000);
  await read();
  assert.equal(mints.length, 1, "reused before expiresAt - 10 s");
  advance(2_000);
  await read();
  assert.equal(mints.length, 2, "re-minted inside the expiry margin");

  const otherUser = { ...fixtureAccess, account: { ...fixtureAccess.account, workosUserId: "user_other" } };
  await bridge.json(otherUser, target, { method: "GET", path: "/v1/app/state" });
  assert.deepEqual(mints.at(-1), { userId: "user_other", runtimeId: "runtime_1" });
  assert.equal(mints.length, 3, "grants are never shared across users");
});

test("a native 401 refreshes the grant once and retries with a fresh signature", async (context) => {
  let stateCalls = 0;
  const world = directWorld(context, {
    "POST /v1/app/actions": () =>
      ++stateCalls === 1
        ? Response.json({ detail: "Not authenticated" }, { status: 401 })
        : Response.json(fixtureState()),
  });
  const { bridge, mints } = fixtureBridge();
  const target = await directTarget(bridge);
  const body = JSON.stringify({ MarkRoomRead: { room_id: "room-1" } });
  await bridge.json(fixtureAccess, target, { method: "POST", path: "/v1/app/actions", body, contentType: "application/json" });

  assert.equal(mints.length, 2);
  const sent = world.agentRequests();
  assert.deepEqual(sent.map((request) => request.headers.get("authorization")), [
    "Bearer native-token-1",
    "Bearer native-token-2",
  ]);
  const assertions = sent.map((request) => request.headers.get(DIRECT_CHAT_AUTHORIZATION_HEADER));
  assert.notEqual(assertions[0], assertions[1], "each attempt carries its own signed event");
  assert.equal(world.signingRequests().length, 2);
});

test("a native 401 after the refresh is unavailable, never a session failure", async (context) => {
  directWorld(context, {
    "GET /v1/app/state": () => new Response(null, { status: 401 }),
  });
  const { bridge, mints } = fixtureBridge();
  const target = await directTarget(bridge);
  const response = await bridge.request(fixtureAccess, target, { method: "GET", path: "/v1/app/state" });
  assert.equal(response.status, 503);
  assert.deepEqual(await response.json(), {
    error: "Chat is unavailable right now. Try again shortly.",
    error_kind: "agent_access_unavailable",
    retryable: true,
  });
  assert.equal(mints.length, 2, "exactly one refresh");
});

test("a bridge authorization rejection is not native expiry and never reads as an expired session", async (context) => {
  silentConsole(context);
  directWorld(context, {
    "GET /v1/app/state": () =>
      Response.json(
        { error: "viewer is not authorized", error_kind: "chat_authorization_invalid", retryable: false },
        { status: 401 }
      ),
  });
  const { bridge, mints } = fixtureBridge();
  const target = await directTarget(bridge);
  await assert.rejects(
    bridge.json(fixtureAccess, target, { method: "GET", path: "/v1/app/state" }),
    (error: unknown) =>
      error instanceof HostedDeviceRequestError &&
      error.status === 403 &&
      error.kind === "chat_authorization_invalid" &&
      error.retryable === false
  );
  assert.equal(mints.length, 1, "no grant refresh for a bridge rejection");
});

test("signatures cover the exact path, method, and body bytes that are sent", async (context) => {
  const world = directWorld(context, {
    "GET /v1/app/state": () => Response.json(fixtureState()),
    "POST /v1/app/actions": () => Response.json(fixtureState()),
  });
  const { bridge } = fixtureBridge();
  const target = await directTarget(bridge);
  await bridge.json(fixtureAccess, target, { method: "GET", path: "/v1/app/state?room_id=room-1&limit=50" });
  const body = JSON.stringify({ SendMessage: { room_id: "room-1", text: "héllo ✓" } });
  await bridge.json(fixtureAccess, target, {
    method: "POST",
    path: "/v1/app/actions",
    body,
    contentType: "application/json",
  });

  const [getSignature, postSignature] = world.signingRequests().map((request) => ({
    headers: request.headers,
    input: JSON.parse(new TextDecoder().decode(request.body!)),
  }));
  assert.deepEqual(getSignature.input, {
    version: 1,
    agent_id: "agent_project_1",
    method: "GET",
    path: "/v1/app/state?room_id=room-1&limit=50",
    body_sha256: null,
  });
  const [getSent, postSent] = world.agentRequests();
  assert.equal(getSent.url, `${BRIDGE_URL}/v1/app/state?room_id=room-1&limit=50`);
  assert.equal(getSent.body, null);

  assert.equal(postSignature.input.method, "POST");
  assert.equal(postSignature.input.path, "/v1/app/actions");
  assert.equal(
    postSignature.input.body_sha256,
    createHash("sha256").update(postSent.body!).digest("hex")
  );
  assert.equal(new TextDecoder().decode(postSent.body!), body);
  // The signer is the hosted device, authorized like every other hosted-device call.
  assert.equal(postSignature.headers.get("authorization"), `Bearer ${HOSTED_API_TOKEN}`);
  assert.equal(postSignature.headers.get("x-finite-workos-user-id"), "user_paul");
  assert(![...postSignature.headers.values()].some((value) => value.includes("native-token")));
});

test("multipart uploads sign the exact bytes and boundary sent to the agent", async (context) => {
  const world = directWorld(context, {
    "POST /v1/app/attachments": () => Response.json(fixtureState()),
  });
  const { bridge } = fixtureBridge();
  const target = await directTarget(bridge);
  const formData = new FormData();
  formData.set("room_id", "room-1");
  formData.set("caption", "Screenshot");
  formData.append("files", new Blob(["binary\u0000bytes"], { type: "application/octet-stream" }), "notes.bin");
  const encoded = await encodeMultipartBody(formData);
  await bridge.json(fixtureAccess, target, {
    method: "POST",
    path: "/v1/app/attachments",
    body: encoded.body,
    contentType: encoded.contentType,
  });

  const [sent] = world.agentRequests();
  const signature = JSON.parse(new TextDecoder().decode(world.signingRequests()[0].body!));
  assert.match(sent.headers.get("content-type") ?? "", /^multipart\/form-data; boundary=/u);
  assert.equal(sent.headers.get("content-type"), encoded.contentType);
  assert.equal(signature.body_sha256, createHash("sha256").update(sent.body!).digest("hex"));
  const text = new TextDecoder().decode(sent.body!);
  const boundary = encoded.contentType.split("boundary=")[1]!;
  assert(text.includes(boundary));
  assert(text.includes("notes.bin"));
  assert(text.includes("Screenshot"));
});

test("bridge requests carry only the native bearer, the signed assertion, and requester context", async (context) => {
  const world = directWorld(context, {
    "POST /v1/app/actions": () => Response.json(fixtureState()),
  });
  const { bridge } = fixtureBridge();
  const target = await directTarget(bridge);
  const requesterFor: string[] = [];
  await bridge.json(fixtureAccess, target, {
    method: "POST",
    path: "/v1/app/actions",
    body: "{}",
    contentType: "application/json",
    requester: async (signer) => {
      requesterFor.push(signer);
      return { email: "paul@finite.vip", sitesAssertion: "sites-assertion" };
    },
  });
  const [sent] = world.agentRequests();
  assert.equal(sent.headers.get("authorization"), "Bearer native-token-1");
  assert.equal(sent.headers.get("x-finite-requester-email"), "paul@finite.vip");
  assert.equal(sent.headers.get("x-finite-sites-requester-assertion"), "sites-assertion");
  assert.deepEqual(requesterFor, ["11".repeat(32)], "requester context is for the signer");

  const header = sent.headers.get(DIRECT_CHAT_AUTHORIZATION_HEADER) ?? "";
  assert.match(header, /^[A-Za-z0-9_-]+$/u, "unpadded base64url");
  const event = JSON.parse(Buffer.from(header, "base64url").toString("utf8"));
  assert.equal(event.kind, 27235);
  assert.equal(event.pubkey, "11".repeat(32));
  assert.deepEqual(event.tags[0], ["u", "finitechat-direct://agent_project_1/v1/app/actions"]);

  for (const request of [sent, ...world.requests.filter((r) => r.url.endsWith("/v1/app/info"))]) {
    const values = [...request.headers.values()].join("\n");
    assert(!values.includes(WORKOS_ACCESS_TOKEN), "the WorkOS token never reaches the agent");
    assert(!values.includes(HOSTED_API_TOKEN), "the hosted-device token never reaches the agent");
    assert.equal(request.headers.get("x-finite-workos-user-id"), null);
    assert.equal(request.headers.get("cookie"), null);
  }
  assert.equal(world.requests.some((request) => request.url.startsWith(`${DEVICE_URL}/v1/app/`)), false);
});

test("bridge paths a URL parser could rewrite are rejected before signing", async (context) => {
  const world = directWorld(context, {});
  const { bridge } = fixtureBridge();
  const target = await directTarget(bridge);
  for (const path of [
    "/v1/app/../admin",
    "/v1/app/attachments/./room/message",
    "/v1/app/attachments/room//attachment",
    "/v1/hermes/state",
    "/v1/app/state#fragment",
    "/v1/app/state\\..\\x",
    `/v1/app/state?${"x".repeat(2050)}`,
  ]) {
    await assert.rejects(
      bridge.request(fixtureAccess, target, { method: "GET", path }),
      (error: unknown) => error instanceof HostedWebChatError && error.status === 400,
      path
    );
  }
  await assert.rejects(
    bridge.request(fixtureAccess, target, { method: "GET", path: "/v1/app/state", body: "{}" }),
    HostedWebChatError
  );
  assert.equal(world.signingRequests().length, 0);
});

test("the canonical room comes from agent info and fails closed when ambiguous", () => {
  const info = classifyDirectChatInfo(directInfo(), "agent_project_1");
  assert.equal(info.mode, "direct");
  if (info.mode !== "direct") return;
  assert.equal(directChatCanonicalRoomId(info.info), "room-1");
  assert.equal(
    directChatCanonicalRoomId({ ...info.info, canonicalRoomId: null, rooms: ["room-only"] }),
    "room-only"
  );
  assert.throws(
    () => directChatCanonicalRoomId({ ...info.info, canonicalRoomId: null, rooms: ["room-a", "room-b"] }),
    (error: unknown) => error instanceof HostedWebChatError && error.status === 409
  );
  assert.throws(
    () => directChatCanonicalRoomId({ ...info.info, canonicalRoomId: null, rooms: [] }),
    (error: unknown) => error instanceof HostedWebChatError && error.status === 503
  );
});

test("agent ids match the Runner's FINITE_AGENT_ID derivation", () => {
  assert.equal(directChatAgentId("project_8d91"), "agent_project_8d91");
  assert.equal(directChatAgentId("a.b/c"), "agent_a-b-c");
  assert.equal(directChatAgentId("-edge-"), "agent_edge");
  assert.equal(directChatAgentId("x".repeat(80)), `agent_${"x".repeat(63)}`);
});

test("agent npubs use NIP-19 bech32", () => {
  assert.equal(npubFromAccountId(AGENT_ACCOUNT_ID), AGENT_NPUB);
  assert.throws(() => npubFromAccountId("not-hex"));
});

test("an oversized info response is not evidence of legacy", async (context) => {
  installFetch(context, () => new Response("x".repeat(70 * 1024)));
  const { bridge } = fixtureBridge();
  await assert.rejects(
    bridge.resolve(fixtureAccess),
    (error: unknown) => error instanceof HostedWebChatError && error.status === 503
  );
});

test("only runtimes the deployment names are probed for direct chat", () => {
  assert.equal(directChatEnabledFor("runtime_a", ""), false);
  assert.equal(directChatEnabledFor("runtime_a", " runtime_b , runtime_a "), true);
  assert.equal(directChatEnabledFor("runtime_c", "runtime_b,runtime_a"), false);
  assert.equal(directChatEnabledFor("runtime_c", "*"), true);
});
