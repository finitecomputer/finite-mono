import assert from "node:assert/strict";
import { test } from "node:test";
import type { AccountAuthContext } from "./dashboard-auth";
import { OBSERVATION_CREDENTIAL_HEADER, type ObservationDependencies } from "./brain-identity-observation";
import { bootstrapHostedWebChatWithContext } from "./hosted-web-chat";
import { HOSTED_OBSERVATION_PATH, HOSTED_OBSERVATION_VERSION, observeHostedAccountIdentity, scheduleHostedAccountObservation } from "./hosted-account-observation";

const account: AccountAuthContext = { email: "human@example.test", workosUserId: "subject-test", emailVerified: true, accessToken: "test-bearer", source: "workos" };
const config = { coreUrl: "http://core.test", credential: "test-observer" };
const key = "ab".repeat(32);
const now = () => new Date("2026-10-27T00:00:00Z");

test("hosted observation registers only the server-loaded key, retries one lost response, and sends no Brain scope", async () => {
  const calls: { url: string; init: RequestInit }[] = [];
  let reads = 0;
  const deps: ObservationDependencies = {
    now, identifyMember: async () => { reads++; return { publicKeyHex: key, npub: "not-used", email: "not-used@example.test" }; },
    fetch: async (input, init) => {
      calls.push({ url: String(input), init: init! });
      if (calls.length === 1) throw new Error("lost response");
      const body = JSON.parse(String(init?.body));
      return Response.json({ version: body.version, operationId: body.operationId, outcome: "recorded" });
    },
  };
  assert.deepEqual(await observeHostedAccountIdentity(config, account, deps), { outcome: "recorded" });
  assert.equal(reads, 1); assert.equal(calls.length, 2);
  assert.equal(calls[0].url, config.coreUrl + HOSTED_OBSERVATION_PATH);
  assert.equal(calls[0].init.body, calls[1].init.body);
  const body = JSON.parse(String(calls[0].init.body));
  assert.deepEqual(Object.keys(body).sort(), ["observedAt", "operationId", "publicKeyHex", "version"]);
  assert.equal(body.publicKeyHex, key); assert.equal(body.version, HOSTED_OBSERVATION_VERSION);
  const headers = new Headers(calls[0].init.headers);
  assert.equal(headers.get("authorization"), "Bearer test-bearer");
  assert.equal(headers.get(OBSERVATION_CREDENTIAL_HEADER), "test-observer");
});

test("missing identity, unverified Accounts, old Core, and failures remain best effort", async () => {
  let calls = 0;
  const deps: ObservationDependencies = { now, identifyMember: async () => ({ publicKeyHex: key }),
    fetch: async () => { calls++; return new Response(null, { status: 404 }); } };
  for (const invalid of [null, {}, { publicKeyHex: key.toUpperCase() }, { publicKeyHex: "short" }]) {
    assert.ok("skipped" in await observeHostedAccountIdentity(config, account, { ...deps, identifyMember: async () => invalid }));
  }
  await observeHostedAccountIdentity(config, account, { ...deps, identifyMember: async () => { throw new Error("unavailable"); } });
  await observeHostedAccountIdentity(null, account, deps);
  for (const invalid of [{ ...account, accessToken: undefined }, { ...account, emailVerified: false }, { ...account, workosUserId: null }]) {
    await observeHostedAccountIdentity(config, invalid, deps);
  }
  assert.equal(calls, 0);
  assert.deepEqual(await observeHostedAccountIdentity(config, account, deps), { skipped: "core refused (404)" });
  for (const status of [409, 503]) {
    assert.ok("skipped" in await observeHostedAccountIdentity(config, account, { ...deps, fetch: async () => new Response(null, { status }) }));
  }
  assert.ok("skipped" in await observeHostedAccountIdentity(config, account, { ...deps, fetch: async () => { throw new Error("down"); } }));
});

test("successful real bootstrap schedules a load-only observation after Chat; failed bootstrap schedules nothing", async context => {
  const names = ["FC_CORE_BRAIN_IDENTITY_URL", "FC_CORE_BRAIN_OBSERVATION_TOKEN", "FC_BRAIN_PUBLIC_ORIGIN"] as const;
  const previous = names.map(name => process.env[name]);
  const originalFetch = global.fetch;
  context.after(() => { global.fetch = originalFetch; names.forEach((name, i) => { if (previous[i] === undefined) delete process.env[name]; else process.env[name] = previous[i]; }); });
  process.env.FC_CORE_BRAIN_IDENTITY_URL = config.coreUrl;
  process.env.FC_CORE_BRAIN_OBSERVATION_TOKEN = config.credential;
  process.env.FC_BRAIN_PUBLIC_ORIGIN = "https://brain.test";
  const queued: (() => Promise<void>)[] = [];
  const calls: string[] = [];
  const chat = { identity: { account_id: key, device_id: "hosted-web" }, rev: 1, rooms: [] };
  let failBootstrap = false;
  global.fetch = async (input, init) => {
    const url = String(input); calls.push(url);
    if (url.endsWith("/agent-bindings/open")) return failBootstrap ? new Response(null, { status: 503 }) : Response.json(chat);
    if (url.endsWith("/brain/identity-provider")) {
      assert.deepEqual(JSON.parse(String(init?.body)), { version: "finite-brain-identity-provider-v1", operation: "identifyMember", input: {} });
      assert.equal(new Headers(init?.headers).get("x-finite-workos-user-id"), account.workosUserId);
      return Response.json({ publicKeyHex: key });
    }
    assert.equal(url, config.coreUrl + HOSTED_OBSERVATION_PATH);
    return new Response(null, { status: 503 });
  };
  const bootstrapContext = { account, config: { baseUrl: "http://hosted.test", apiToken: "test-device" }, trialAccount: false, primaryUrl: "http://agent.test", agentName: "Agent", projectId: "project-test", runtimeId: "runtime-test" };
  const schedule = (a: AccountAuthContext, d: typeof bootstrapContext.config) => scheduleHostedAccountObservation(a, d, work => { queued.push(work); });
  assert.deepEqual(await bootstrapHostedWebChatWithContext(bootstrapContext, schedule), chat);
  assert.equal(calls.length, 1); assert.equal(queued.length, 1);
  await queued.pop()!(); // Even a Core outage cannot alter the delivered Chat state.
  assert.equal(calls.length, 4);
  failBootstrap = true;
  await assert.rejects(bootstrapHostedWebChatWithContext(bootstrapContext, schedule));
  assert.equal(queued.length, 0);
  failBootstrap = false;
  assert.deepEqual(await bootstrapHostedWebChatWithContext(bootstrapContext, (a, d) =>
    scheduleHostedAccountObservation(a, d, () => { throw new Error("scheduler closed"); })), chat);
});
