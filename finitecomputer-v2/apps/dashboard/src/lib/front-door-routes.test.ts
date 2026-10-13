import assert from "node:assert/strict";
import { once } from "node:events";
import { readFile } from "node:fs/promises";
import { createServer } from "node:http";
import test from "node:test";

import { POST as createAgent } from "@/app/dashboard/agent-creation-requests/route";
import { GET as openWayBack, POST as savePreference } from "@/app/dashboard/front-door/route";
import {
  FRONT_DOOR_FRESH_COOKIE,
  FRONT_DOOR_STAY_COOKIE,
  frontDoorAgentCreationMovedMessage,
  frontDoorConfig,
} from "./front-door";

async function fixtureCore(t: test.TestContext, status = 200) {
  const calls: string[] = [];
  const bodies: unknown[] = [];
  const core = createServer(async (request, response) => {
    calls.push(`${request.method} ${request.url}`);
    let body = "";
    for await (const chunk of request) body += chunk;
    if (body) bodies.push(JSON.parse(body));
    response.statusCode = status;
    response.setHeader("content-type", "application/json");
    response.end(status === 200 ? "{}" : JSON.stringify({ error: "Core is unavailable" }));
  });
  core.listen(0, "127.0.0.1");
  await once(core, "listening");
  t.after(() => {
    core.closeAllConnections();
    core.close();
  });
  const address = core.address();
  assert(address && typeof address !== "string");
  return { calls, bodies, baseUrl: `http://127.0.0.1:${address.port}` };
}

const SIGNED_IN_ENV = {
  NEXT_PUBLIC_WORKOS_REDIRECT_URI: "https://finite.computer/callback",
  FC_CORE_API_TOKEN: "fixture",
  FC_WORKOS_AUTH_ENABLED: "0",
  FC_DASHBOARD_ALLOW_DEV_ACCOUNT_AUTH: "1",
  FC_DASHBOARD_DEV_EMAIL: "owner@example.test",
  FC_DASHBOARD_DEV_WORKOS_USER_ID: "user_owner",
  FC_DASHBOARD_DEV_WORKOS_ACCESS_TOKEN: "fixture-access-token",
  WORKOS_COOKIE_PASSWORD: "front-door-fixture-cookie-password-32-chars",
};

function preferenceForm(value: string) {
  const form = new FormData();
  form.set("useNewFinite", value);
  return new Request("https://finite.computer/dashboard/front-door", { method: "POST", body: form });
}

function cookieHeader(response: Response, name: string) {
  return (response.headers.get("set-cookie") ?? "")
    .split(/,(?=\s*[^;]+=)/u)
    .map((cookie) => cookie.trim())
    .find((cookie) => cookie.startsWith(`${name}=`));
}

function stayCookieHeader(response: Response) {
  return cookieHeader(response, FRONT_DOOR_STAY_COOKIE);
}

function creationForm() {
  const form = new FormData();
  form.set("displayName", "Oslo Bot");
  form.set("hostingTier", "standard");
  form.set("idempotencyKey", "front-door-test-key");
  return new Request("https://finite.computer/dashboard/agent-creation-requests", {
    method: "POST",
    body: form,
  });
}

test("agent creation POST refuses with a clear message once the front door is on", async (t) => {
  const saved = { ...process.env };
  t.after(() => {
    process.env = saved;
  });
  const core = await fixtureCore(t);
  Object.assign(process.env, {
    FC_FRONT_DOOR_ENABLED: "true",
    NEXT_PUBLIC_WORKOS_REDIRECT_URI: "https://finite.computer/callback",
    FC_CORE_BASE_URL: core.baseUrl,
    FC_CORE_API_TOKEN: "fixture",
    FC_WORKOS_AUTH_ENABLED: "0",
    FC_DASHBOARD_ALLOW_DEV_ACCOUNT_AUTH: "1",
    FC_DASHBOARD_DEV_EMAIL: "owner@example.test",
    FC_DASHBOARD_DEV_WORKOS_USER_ID: "user_owner",
    FC_DASHBOARD_DEV_WORKOS_ACCESS_TOKEN: "fixture-access-token",
    WORKOS_COOKIE_PASSWORD: "front-door-fixture-cookie-password-32-chars",
  });

  const response = await createAgent(creationForm());
  assert.equal(response.status, 303);
  const location = new URL(response.headers.get("location") ?? "");
  assert.equal(location.origin, "https://finite.computer");
  assert.equal(location.pathname, "/dashboard");
  assert.equal(
    location.searchParams.get("agentCreationError"),
    frontDoorAgentCreationMovedMessage(frontDoorConfig())
  );
  assert.match(location.searchParams.get("agentCreationError") ?? "", /finite\.chat/u);
  assert.equal(response.headers.get("set-cookie"), null, "the saved draft is untouched");
  assert.equal(core.calls.length, 0, "no billing read, checkout or Core creation happens");

  // Operators keep creation with the switch on (rollouts need fresh agents).
  process.env.FC_DASHBOARD_DEV_ADMIN_EMAILS = "owner@example.test";
  const operator = await createAgent(creationForm());
  assert.equal(operator.status, 303);
  assert.notEqual(
    new URL(operator.headers.get("location") ?? "").searchParams.get("agentCreationError"),
    frontDoorAgentCreationMovedMessage(frontDoorConfig())
  );
  assert.ok(core.calls.includes("GET /api/core/v1/me/billing"), core.calls.join(", "));
  process.env.FC_DASHBOARD_DEV_ADMIN_EMAILS = "";
  core.calls.length = 0;

  // Switch off: the same submission takes today's path (it reaches Core's
  // billing read instead of being refused).
  process.env.FC_FRONT_DOOR_ENABLED = "false";
  const unchanged = await createAgent(creationForm());
  assert.equal(unchanged.status, 303);
  assert.notEqual(
    new URL(unchanged.headers.get("location") ?? "").searchParams.get("agentCreationError"),
    frontDoorAgentCreationMovedMessage(frontDoorConfig({ FC_FRONT_DOOR_ENABLED: "true" }))
  );
  assert.ok(core.calls.includes("GET /api/core/v1/me/billing"), core.calls.join(", "));
});

test("signup hands off to the new Finite before any WorkOS sign-up work", async () => {
  // The route imports the AuthKit SDK, which Node's test loader cannot
  // resolve; its configuration is covered by front-door.test.ts. Pin the
  // ordering here: an enabled switch redirects before WorkOS is consulted,
  // and a disabled one falls through to today's WorkOS sign-up unchanged.
  const source = await readFile(new URL("../app/signup/route.ts", import.meta.url), "utf8");
  const handoff = source.indexOf("redirect(frontDoor.signupUrl)");
  assert.ok(source.includes("const frontDoor = loadFrontDoorConfig();"));
  assert.ok(source.includes("if (frontDoor.enabled) {"));
  assert.ok(handoff > 0, "the enabled switch redirects to the configured sign-up URL");
  assert.ok(handoff < source.indexOf("workosAuthStatus()"));
  assert.ok(handoff < source.indexOf("getSignUpUrl({"));
  assert.ok(source.includes("returnTo: safeWorkosReturnPathname("), "legacy sign-up is unchanged");
});

test("the way back opens legacy for the session and the setting turns on and off", async (t) => {
  const saved = { ...process.env };
  t.after(() => {
    process.env = saved;
  });
  const core = await fixtureCore(t);
  Object.assign(process.env, SIGNED_IN_ENV, {
    FC_FRONT_DOOR_ENABLED: "true",
    FC_CORE_BASE_URL: core.baseUrl,
  });

  // GET: a session "stay" marker scoped to the dashboard, then the setting.
  const visit = await openWayBack(new Request("https://finite.computer/dashboard/front-door"));
  assert.equal(visit.status, 303);
  assert.equal(visit.headers.get("location"), "https://finite.computer/dashboard#front-door");
  const marker = stayCookieHeader(visit) ?? "";
  assert.match(marker, new RegExp(`^${FRONT_DOOR_STAY_COOKIE}=1;`, "u"));
  assert.match(marker, /Path=\/dashboard/u);
  assert.match(marker, /HttpOnly/u);
  assert.match(marker, /Max-Age=43200/u);
  assert.equal(core.calls.length, 0, "opening the way back changes nothing in Core");

  // POST on: Core records the caller's own preference; the layout decides next.
  const on = await savePreference(preferenceForm("true"));
  assert.equal(on.status, 303);
  assert.equal(on.headers.get("location"), "https://finite.computer/dashboard");
  assert.match(stayCookieHeader(on) ?? "", /Max-Age=0/u, "the choice replaces the visit");
  // The next loads read /me uncached so the change takes effect at once.
  const fresh = cookieHeader(on, FRONT_DOOR_FRESH_COOKIE) ?? "";
  assert.match(fresh, new RegExp(`^${FRONT_DOOR_FRESH_COOKIE}=1;`, "u"));
  assert.match(fresh, /Path=\/dashboard/u);
  assert.match(fresh, /Max-Age=30/u);
  // POST off: back on legacy, landing on the setting.
  const off = await savePreference(preferenceForm("false"));
  assert.equal(off.headers.get("location"), "https://finite.computer/dashboard#front-door");
  assert.match(stayCookieHeader(off) ?? "", /Max-Age=0/u);
  assert.match(cookieHeader(off, FRONT_DOOR_FRESH_COOKIE) ?? "", /Max-Age=30/u);
  assert.deepEqual(core.calls, [
    "PUT /api/core/v1/me/front-door-preference",
    "PUT /api/core/v1/me/front-door-preference",
  ]);
  assert.deepEqual(core.bodies, [{ useNewFinite: true }, { useNewFinite: false }]);

  // A malformed value never reaches Core and keeps any visit marker.
  for (const value of ["yes", "", "TRUE"]) {
    const rejected = await savePreference(preferenceForm(value));
    assert.equal(
      rejected.headers.get("location"),
      "https://finite.computer/dashboard?frontDoorError=1#front-door"
    );
    assert.equal(stayCookieHeader(rejected), undefined);
  }
  assert.equal(core.calls.length, 2);
});

test("a preference Core cannot save is reported, not assumed", async (t) => {
  const saved = { ...process.env };
  t.after(() => {
    process.env = saved;
  });
  const core = await fixtureCore(t, 503);
  Object.assign(process.env, SIGNED_IN_ENV, {
    FC_FRONT_DOOR_ENABLED: "true",
    FC_CORE_BASE_URL: core.baseUrl,
  });
  const failed = await savePreference(preferenceForm("true"));
  assert.equal(
    failed.headers.get("location"),
    "https://finite.computer/dashboard?frontDoorError=1#front-door"
  );
  assert.equal(stayCookieHeader(failed), undefined, "the visit marker survives a failed save");
  assert.deepEqual(core.calls, ["PUT /api/core/v1/me/front-door-preference"]);
});

test("switched off, the way back does not exist and touches nothing", async (t) => {
  const saved = { ...process.env };
  t.after(() => {
    process.env = saved;
  });
  const core = await fixtureCore(t);
  Object.assign(process.env, SIGNED_IN_ENV, {
    FC_FRONT_DOOR_ENABLED: "false",
    FC_CORE_BASE_URL: core.baseUrl,
  });
  for (const response of [
    await openWayBack(new Request("https://finite.computer/dashboard/front-door")),
    await savePreference(preferenceForm("true")),
  ]) {
    assert.equal(response.status, 404);
    assert.equal(response.headers.get("set-cookie"), null);
  }
  assert.equal(core.calls.length, 0);
});
