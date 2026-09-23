import assert from "node:assert/strict";
import test from "node:test";
import { HostedHermesStatusError, parseHostedHermesStatus } from "./hosted-hermes-status";

test("native status selects version and gateway state without treating a stopped gateway as unreachable", () => {
  assert.deepEqual(parseHostedHermesStatus({
    version: "0.21.0", gateway_running: false, install_id: "not-for-display", unknown: { path: "private" },
  }), { version: "0.21.0", gatewayRunning: false });
  for (const invalid of [null, {}, { version: "", gateway_running: true }, { version: "1", gateway_running: "true" }]) {
    assert.throws(() => parseHostedHermesStatus(invalid));
  }
});

import { readHostedHermesJson, parseHostedHermesSession } from "./hosted-hermes-status";

test("agent read reauthorizes once on expiry without forwarding app cookies", async (t) => {
  const calls: {url: string; init: RequestInit}[] = [];
  t.mock.method(globalThis, "fetch", async (input: string | URL, init: RequestInit) => {
    const url = String(input);
    calls.push({url,init});
    if (url.startsWith("/api/agents/")) return Response.json({baseUrl:"https://agents.test/runtimes/a/",accessToken:`synthetic-${calls.length}`,expiresAt:Math.floor(Date.now()/1000)+60});
    return calls.length === 2 ? new Response(null,{status:401}) : Response.json({ok:true});
  });
  assert.deepEqual(await readHostedHermesJson("a","api/auth/me",new AbortController().signal),{ok:true});
  assert.equal(calls.length,4);
  assert.equal(calls[0].init.method,"POST");
  assert.equal(calls[2].init.method,"POST");
  for (const call of [calls[1],calls[3]]) {
    assert.equal(call.url,"https://agents.test/runtimes/a/api/auth/me");
    assert.equal(call.init.credentials,"omit");
    assert.equal(call.init.redirect,"error");
    assert.equal(call.init.cache,"no-store");
    assert.ok(new Headers(call.init.headers).get("authorization")?.startsWith("Bearer synthetic-"));
  }
});

test("account denial never contacts agent and response errors are sanitized", async (t) => {
  let count=0;
  t.mock.method(globalThis,"fetch",async()=>{count++;return Response.json({secret:"must-not-escape"},{status:403});});
  await assert.rejects(readHostedHermesJson("a","api/auth/me",new AbortController().signal),/unavailable for this account/);
  assert.equal(count,1);
});

test("agent switch cancellation rejects a late grant before sending credentials", async (t) => {
  const controller=new AbortController();
  let count=0;
  t.mock.method(globalThis,"fetch",async()=>{
    count++;
    controller.abort();
    return Response.json({baseUrl:"https://agents.test/runtimes/a/",accessToken:"synthetic",expiresAt:Math.floor(Date.now()/1000)+60});
  });
  await assert.rejects(readHostedHermesJson("a","api/auth/me",controller.signal));
  assert.equal(count,1);
});

test("grant validation projects safe fields independently of the device clock",()=>{
  const grant={baseUrl:"https://agents.test/runtimes/a/",accessToken:"synthetic",expiresAt:Math.floor(Date.now()/1000)+60};
  assert.deepEqual(parseHostedHermesSession({...grant,password:"never-forwarded"}),grant);
  assert.equal(parseHostedHermesSession({...grant,expiresAt:grant.expiresAt-3600}).expiresAt,grant.expiresAt-3600);
  assert.equal(parseHostedHermesSession({...grant,expiresAt:grant.expiresAt+3600}).expiresAt,grant.expiresAt+3600);
  for(const baseUrl of ["http://agents.test/","https://user:password@agents.test/","https://agents.test/?token=x","https://agents.test/#token"]) {
    assert.throws(()=>parseHostedHermesSession({...grant,baseUrl}));
  }
  for(const expiresAt of [0,-1,1.5,Infinity]) assert.throws(()=>parseHostedHermesSession({...grant,expiresAt}));
});

test("read rejects arbitrary URLs and oversized native responses", async (t)=>{
  let calls=0;
  t.mock.method(globalThis,"fetch",async()=>{
    calls++;
    return calls===1?Response.json({baseUrl:"https://agents.test/runtimes/a/",accessToken:"synthetic",expiresAt:Math.floor(Date.now()/1000)+60}):new Response("x".repeat(1024*1024+1));
  });
  for(const path of ["https://other.test/api/auth/me","//other.test/","api/../auth/me","api/auth/me?x=1","api/skills?inventory=true&profile=other","api/skills?inventory=false"]) await assert.rejects(readHostedHermesJson("a",path,new AbortController().signal),/Invalid agent API path/);
  assert.equal(calls,0);
  await assert.rejects(readHostedHermesJson("a","api/auth/me",new AbortController().signal),/too large/);
});


test("malformed, empty and oversized native bodies invalidate prior inventory", async (t) => {
  let body: string | null = null;
  t.mock.method(globalThis, "fetch", async (input: string | URL) => String(input).startsWith("/api/agents/")
    ? Response.json({ baseUrl: "https://agents.test/a/", accessToken: "synthetic", expiresAt: 100 })
    : new Response(body));
  for (body of [null, "", "not JSON", "x".repeat(1024 * 1024 + 1)]) {
    await assert.rejects(readHostedHermesJson("a", "api/plugins/finite-brain/overview", new AbortController().signal),
      error => error instanceof HostedHermesStatusError && error.kind === "unsupported");
  }
});


test("each native chat connection obtains a single-use ticket without exposing its grant in the URL", async (t) => {
  const { createHostedHermesWebSocket } = await import("./hosted-hermes-status");
  const calls: { url: string; init: RequestInit }[] = [];
  t.mock.method(globalThis, "fetch", async (input: string | URL, init: RequestInit) => {
    calls.push({ url: String(input), init });
    return calls.length % 2 === 1
      ? Response.json({ baseUrl: "https://agents.test/runtimes/a/", accessToken: "private-grant", expiresAt: 100 })
      : Response.json({ ticket: `one-use-${calls.length}` });
  });
  for (const suffix of [2, 4]) {
    assert.deepEqual(await createHostedHermesWebSocket("a", new AbortController().signal),
      { url: "wss://agents.test/runtimes/a/api/ws", protocols: ["hermes-gateway-v1", `hermes-gateway-ticket.one-use-${suffix}`] });
  }
  assert.deepEqual(calls.map(c => c.url), [
    "/api/agents/a/hermes-access", "https://agents.test/runtimes/a/api/auth/ws-ticket",
    "/api/agents/a/hermes-access", "https://agents.test/runtimes/a/api/auth/ws-ticket",
  ]);
  assert.equal(new Headers(calls[1].init.headers).get("authorization"), "Bearer private-grant");
  assert.equal(calls[1].init.credentials, "omit");
  assert.equal(calls[1].init.redirect, "error");
});

test("native chat cancels a late grant on agent switch and preserves account 401 classification", async (t) => {
  const { createHostedHermesWebSocket } = await import("./hosted-hermes-status");
  const controller = new AbortController();
  let calls = 0;
  const mock = t.mock.method(globalThis, "fetch", async () => {
    calls++;
    controller.abort();
    return Response.json({ baseUrl: "https://agents.test/a/", accessToken: "private-grant", expiresAt: 100 });
  });
  await assert.rejects(createHostedHermesWebSocket("a", controller.signal));
  assert.equal(calls, 1);
  mock.mock.mockImplementation(async () => Response.json({}, { status: 401 }));
  await assert.rejects(createHostedHermesWebSocket("a", new AbortController().signal),
    e => e instanceof HostedHermesStatusError && e.status === 401);
});


test("archive uses a fresh account grant and never retries a refused mutation", async (t) => {
  const { setHostedHermesSessionArchived } = await import("./hosted-hermes-status");
  const calls: { url: string; init: RequestInit }[] = [];
  t.mock.method(globalThis, "fetch", async (input: string | URL, init: RequestInit) => {
    calls.push({ url: String(input), init });
    return calls.length === 1
      ? Response.json({ baseUrl: "https://agents.test/runtimes/a/", accessToken: "synthetic", expiresAt: 100 })
      : new Response(null, { status: 401 });
  });
  await assert.rejects(setHostedHermesSessionArchived("a", "session-1", true, new AbortController().signal), /no longer available/);
  assert.equal(calls.length, 2);
  assert.equal(calls[1].url, "https://agents.test/runtimes/a/api/sessions/session-1");
  assert.equal(calls[1].init.method, "PATCH");
  assert.equal(calls[1].init.credentials, "omit");
  assert.equal(calls[1].init.redirect, "error");
  assert.deepEqual(JSON.parse(String(calls[1].init.body)), { archived: true });
});
