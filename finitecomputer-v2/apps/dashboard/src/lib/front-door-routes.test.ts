import assert from "node:assert/strict";
import { once } from "node:events";
import { readFile } from "node:fs/promises";
import { createServer } from "node:http";
import test from "node:test";

import { POST as createAgent } from "@/app/dashboard/agent-creation-requests/route";
import { frontDoorAgentCreationMovedMessage, frontDoorConfig } from "./front-door";

async function fixtureCore(t: test.TestContext) {
  const calls: string[] = [];
  const core = createServer((request, response) => {
    calls.push(`${request.method} ${request.url}`);
    response.setHeader("content-type", "application/json");
    response.end("{}");
  });
  core.listen(0, "127.0.0.1");
  await once(core, "listening");
  t.after(() => {
    core.closeAllConnections();
    core.close();
  });
  const address = core.address();
  assert(address && typeof address !== "string");
  return { calls, baseUrl: `http://127.0.0.1:${address.port}` };
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
