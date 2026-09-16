import assert from "node:assert/strict";
import test from "node:test";
import { parseHostedHermesStatus } from "./hosted-hermes-status";

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
  for(const path of ["https://other.test/api/auth/me","//other.test/","api/../auth/me","api/auth/me?x=1"]) await assert.rejects(readHostedHermesJson("a",path,new AbortController().signal),/Invalid agent API path/);
  assert.equal(calls,0);
  await assert.rejects(readHostedHermesJson("a","api/auth/me",new AbortController().signal),/too large/);
});
