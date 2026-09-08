import assert from "node:assert/strict";
import { test } from "node:test";
import { gateReturnPath, gateSiteOrigin, mintSiteGateSession } from "./site-gate";
import type { AccountAuthContext } from "./dashboard-auth";

const account: AccountAuthContext = { email: "friend@example.com", workosUserId: "user_friend", emailVerified: true, source: "workos" };
const env = { NODE_ENV: "production" as const, FC_SITES_AUTH_GATE_URL: "https://auth.finite.site", FINITE_GATE_ACCOUNT_TOKEN: "ab".repeat(32) };

test("gate targets are restricted to site origins and ordinary return paths", () => {
  assert.equal(gateSiteOrigin("https://hello.finite.site"), "https://hello.finite.site");
  for (const target of ["https://finite.site", "https://auth.finite.site", "https://hello.finite.site.evil.test", "https://a.b.finite.site", "http://hello.finite.site", "https://hello.finite.site:8443", "https://user@hello.finite.site", "https://hello.finite.site/path"]) {
    assert.throws(() => gateSiteOrigin(target));
  }
  assert.equal(gateReturnPath("/report?q=one#section"), "/report?q=one#section");
  for (const path of ["//evil.test", "/\\evil.test", "/_finite/logout", "/../_finite/auth", "/a b"]) assert.throws(() => gateReturnPath(path));
});

test("existing verified account mints one proof with no login or permission lookup", async (t) => {
  const calls: string[] = [];
  t.mock.method(globalThis, "fetch", async (url: URL, options: RequestInit) => {
    calls.push(url.toString());
    assert.equal(options.redirect, "error");
    assert.equal((options.headers as Record<string, string>).authorization, `Bearer ${env.FINITE_GATE_ACCOUNT_TOKEN}`);
    assert.deepEqual(JSON.parse(String(options.body)), { output: "https://hello.finite.site", return_to: "/report?q=1", email: "friend@example.com" });
    return Response.json({ redeem_url: "https://hello.finite.site/_finite/auth?gate_code=proof&return_to=%2Freport%3Fq%3D1" });
  });
  assert.match(await mintSiteGateSession(account, "https://hello.finite.site", "/report?q=1", env), /gate_code=proof/);
  assert.deepEqual(calls, ["https://auth.finite.site/vouch"]);
});

test("unverified, header-derived, anonymous and production dev identities never reach gate", async (t) => {
  const fetch = t.mock.method(globalThis, "fetch", async () => { throw new Error("must not call"); });
  for (const bad of [{ ...account, emailVerified: false }, { ...account, workosUserId: null }, { ...account, source: "header" as const }, { ...account, source: "dev" as const }]) {
    await assert.rejects(mintSiteGateSession(bad, "https://hello.finite.site", "/", env), /Sign in/);
  }
  assert.equal(fetch.mock.callCount(), 0);
});

test("wrong-origin, wrong-path and failed gate replies cannot redirect the browser", async (t) => {
  let reply = Response.json({ redeem_url: "https://evil.test/_finite/auth?gate_code=proof&return_to=%2F" });
  t.mock.method(globalThis, "fetch", async () => reply);
  await assert.rejects(mintSiteGateSession(account, "https://hello.finite.site", "/", env));
  reply = Response.json({ redeem_url: "https://hello.finite.site/_finite/auth?gate_code=proof&return_to=%2Fother" });
  await assert.rejects(mintSiteGateSession(account, "https://hello.finite.site", "/", env));
  reply = new Response("unavailable", { status: 503 });
  await assert.rejects(mintSiteGateSession(account, "https://hello.finite.site", "/", env));
});
